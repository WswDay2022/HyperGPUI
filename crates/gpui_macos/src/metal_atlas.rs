use anyhow::{Context as _, Result};
use collections::FxHashMap;
use derive_more::{Deref, DerefMut};
use etagere::BucketedAtlasAllocator;
use hgpui::{
    AtlasKey, AtlasTextureId, AtlasTextureKind, AtlasTextureList, AtlasTile, Bounds, DevicePixels,
    PlatformAtlas, Point, Size, atlas_page_size,
};
use block::ConcreteBlock;
use metal::{CommandQueue, Device};
use parking_lot::Mutex;
use std::{borrow::Cow, sync::Arc};

// 原实现：pub(crate) struct MetalAtlas(Mutex<MetalAtlasState>);
// A retirement callback owns the state until earlier GPU reads have completed, including close.
pub(crate) struct MetalAtlas(Arc<Mutex<MetalAtlasState>>, AssertSend<CommandQueue>);

impl MetalAtlas {
    // 原实现：new(device, is_apple_gpu) only stored a Mutex; retirement needs the render queue.
    pub(crate) fn new(device: Device, is_apple_gpu: bool, command_queue: CommandQueue) -> Self {
        MetalAtlas(Arc::new(Mutex::new(MetalAtlasState {
            device: AssertSend(device),
            is_apple_gpu,
            monochrome_textures: Default::default(),
            polychrome_textures: Default::default(),
            tiles_by_key: Default::default(),
        })), AssertSend(command_queue))
    }

    pub(crate) fn metal_texture(&self, id: AtlasTextureId) -> metal::Texture {
        self.0.lock().texture(id).metal_texture.clone()
    }

    /// Return the production retirement submission so tests can wait on its exact boundary.
    /// Normal drawing drops this handle without blocking the UI thread.
    fn retire_tiles(&self, keys: &[AtlasKey]) -> Option<metal::CommandBuffer> {
        let tiles: Vec<_> = {
            let mut state = self.0.lock();
            keys.iter().filter_map(|key| state.tiles_by_key.remove(key)).collect()
        };
        if tiles.is_empty() {
            return None;
        }
        // CPU replace_region uploads are not queue-ordered. Removing a key is safe now, but
        // allocator/page reuse must wait for all previously committed draws on this queue.
        // Keep live_atlas_keys and the owning state until completion, even if the window closes.
        let state = self.0.clone();
        let command_buffer = self.1.new_command_buffer();
        command_buffer.set_label("retire shadow atlas tiles");
        // Encode an actual GPU queue marker rather than relying on an empty buffer fence.
        let boundary = self.0.lock().device.new_event();
        command_buffer.encode_signal_event(&boundary, 1);
        let completion = ConcreteBlock::new(move |_| {
            let mut state = state.lock();
            for tile in &tiles {
                state.deallocate_tile(*tile);
            }
        }).copy();
        command_buffer.add_completed_handler(&completion);
        command_buffer.commit();
        Some(command_buffer.to_owned())
    }
}

struct MetalAtlasState {
    device: AssertSend<Device>,
    is_apple_gpu: bool,
    monochrome_textures: AtlasTextureList<MetalAtlasTexture>,
    polychrome_textures: AtlasTextureList<MetalAtlasTexture>,
    tiles_by_key: FxHashMap<AtlasKey, AtlasTile>,
}

impl PlatformAtlas for MetalAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        let mut lock = self.0.lock();
        if let Some(tile) = lock.tiles_by_key.get(key) {
            Ok(Some(*tile))
        } else {
            let Some((size, bytes)) = build()? else {
                return Ok(None);
            };
            let tile = lock
                .allocate(size, key.texture_kind())
                .context("failed to allocate")?;
            let texture = lock.texture(tile.texture_id);
            texture.upload(tile.bounds, &bytes);
            lock.tiles_by_key.insert(key.clone(), tile);
            Ok(Some(tile))
        }
    }

    fn remove(&self, key: &AtlasKey) {
        let mut lock = self.0.lock();
        let Some(tile) = lock.tiles_by_key.remove(key) else {
            return;
        };
        // 原实现：此处按 texture_id 查页，deallocate(tile_id)、decrement_ref_count，
        // 最后一个引用释放时回收 slot；同一实现保留在 deallocate_tile 供完成回调复用。
        lock.deallocate_tile(tile);
    }

    fn retire(&self, keys: &[AtlasKey]) {
        drop(self.retire_tiles(keys));
    }
}

impl MetalAtlasState {
    // This is the original remove deallocation algorithm, shared with delayed retirement.
    fn deallocate_tile(&mut self, tile: AtlasTile) {
        let id = tile.texture_id;
        let textures = match id.kind {
            AtlasTextureKind::Monochrome => &mut self.monochrome_textures,
            AtlasTextureKind::Polychrome => &mut self.polychrome_textures,
            AtlasTextureKind::Subpixel => unreachable!(),
        };
        let Some(texture_slot) = textures.textures.iter_mut()
            .find(|texture| texture.as_ref().is_some_and(|v| v.id == id)) else {
            return;
        };
        if let Some(mut texture) = texture_slot.take() {
            texture.allocator.deallocate(tile.tile_id.into());
            texture.decrement_ref_count();
            if texture.is_unreferenced() {
                textures.free_list.push(id.index as usize);
            } else {
                *texture_slot = Some(texture);
            }
        }
    }

    fn allocate(
        &mut self,
        size: Size<DevicePixels>,
        texture_kind: AtlasTextureKind,
    ) -> Option<AtlasTile> {
        // Modern Metal devices support at most 16384 in either texture dimension.
        let page_size = atlas_page_size(size, 16384)?;
        {
            let textures = match texture_kind {
                AtlasTextureKind::Monochrome => &mut self.monochrome_textures,
                AtlasTextureKind::Polychrome => &mut self.polychrome_textures,
                AtlasTextureKind::Subpixel => unreachable!(),
            };

            if let Some(tile) = textures
                .iter_mut()
                .rev()
                .find_map(|texture| texture.allocate(size))
            {
                return Some(tile);
            }
        }

        // Previously: self.push_texture(size, texture_kind).allocate(size)
        // The old path published an empty page before knowing whether the first tile fitted.
        self.allocate_new_texture(size, page_size, texture_kind)
    }

    fn allocate_new_texture(
        &mut self,
        tile_size: Size<DevicePixels>,
        size: Size<DevicePixels>,
        kind: AtlasTextureKind,
    ) -> Option<AtlasTile> {
        // Previously: min_size.min(&MAX_ATLAS_SIZE).max(&DEFAULT_ATLAS_SIZE)
        // `size` is now validated by atlas_page_size without shrinking the tile demand.
        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(size.width.into());
        texture_descriptor.set_height(size.height.into());
        let pixel_format;
        let usage;
        match kind {
            AtlasTextureKind::Monochrome => {
                pixel_format = metal::MTLPixelFormat::A8Unorm;
                usage = metal::MTLTextureUsage::ShaderRead;
            }
            AtlasTextureKind::Polychrome => {
                pixel_format = metal::MTLPixelFormat::BGRA8Unorm;
                usage = metal::MTLTextureUsage::ShaderRead;
            }
            AtlasTextureKind::Subpixel => unreachable!(),
        }
        texture_descriptor.set_pixel_format(pixel_format);
        texture_descriptor.set_usage(usage);
        // Shared memory mode can be used only on Apple GPU families
        // https://developer.apple.com/documentation/metal/mtlresourceoptions/storagemodeshared
        texture_descriptor.set_storage_mode(if self.is_apple_gpu {
            metal::MTLStorageMode::Shared
        } else {
            metal::MTLStorageMode::Managed
        });
        let metal_texture = self.device.new_texture(&texture_descriptor);

        let texture_list = match kind {
            AtlasTextureKind::Monochrome => &mut self.monochrome_textures,
            AtlasTextureKind::Polychrome => &mut self.polychrome_textures,
            AtlasTextureKind::Subpixel => unreachable!(),
        };

        texture_list.try_insert_with(|index| {
            let mut atlas_texture = MetalAtlasTexture {
                id: AtlasTextureId {
                    index: index as u32,
                    kind,
                },
                allocator: etagere::BucketedAtlasAllocator::new(size_to_etagere(size)),
                metal_texture: AssertSend(metal_texture),
                live_atlas_keys: 0,
            };
            let tile = atlas_texture.allocate(tile_size)?;
            Some((atlas_texture, tile))
        })
    }

    fn texture(&self, id: AtlasTextureId) -> &MetalAtlasTexture {
        let textures = match id.kind {
            AtlasTextureKind::Monochrome => &self.monochrome_textures,
            AtlasTextureKind::Polychrome => &self.polychrome_textures,
            AtlasTextureKind::Subpixel => unreachable!(),
        };
        textures[id.index as usize].as_ref().unwrap()
    }
}

struct MetalAtlasTexture {
    id: AtlasTextureId,
    allocator: BucketedAtlasAllocator,
    metal_texture: AssertSend<metal::Texture>,
    live_atlas_keys: u32,
}

impl MetalAtlasTexture {
    fn allocate(&mut self, size: Size<DevicePixels>) -> Option<AtlasTile> {
        let allocation = self.allocator.allocate(size_to_etagere(size))?;
        let tile = AtlasTile {
            texture_id: self.id,
            tile_id: allocation.id.into(),
            bounds: Bounds {
                origin: point_from_etagere(allocation.rectangle.min),
                size,
            },
            padding: 0,
        };
        self.live_atlas_keys += 1;
        Some(tile)
    }

    fn upload(&self, bounds: Bounds<DevicePixels>, bytes: &[u8]) {
        let region = metal::MTLRegion::new_2d(
            bounds.origin.x.into(),
            bounds.origin.y.into(),
            bounds.size.width.into(),
            bounds.size.height.into(),
        );
        self.metal_texture.replace_region(
            region,
            0,
            bytes.as_ptr() as *const _,
            bounds.size.width.to_bytes(self.bytes_per_pixel()) as u64,
        );
    }

    fn bytes_per_pixel(&self) -> u8 {
        use metal::MTLPixelFormat::*;
        match self.metal_texture.pixel_format() {
            A8Unorm | R8Unorm => 1,
            RGBA8Unorm | BGRA8Unorm => 4,
            _ => unimplemented!(),
        }
    }

    fn decrement_ref_count(&mut self) {
        self.live_atlas_keys -= 1;
    }

    fn is_unreferenced(&mut self) -> bool {
        self.live_atlas_keys == 0
    }
}

fn size_to_etagere(size: Size<DevicePixels>) -> etagere::Size {
    etagere::Size::new(size.width.into(), size.height.into())
}

fn point_from_etagere(value: etagere::Point) -> Point<DevicePixels> {
    Point {
        x: DevicePixels::from(value.x),
        y: DevicePixels::from(value.y),
    }
}

#[derive(Deref, DerefMut)]
struct AssertSend<T>(T);

unsafe impl<T> Send for AssertSend<T> {}

#[cfg(test)]
mod tests {
    // Previously: use super::*; Explicit imports keep hgpui's test macro out of scope.
    use super::MetalAtlas;
    use hgpui::{AtlasKey, AtlasTextureKind, AtlasTile, DevicePixels, PlatformAtlas, Size};
    use std::borrow::Cow;

    fn create_atlas() -> Option<MetalAtlas> {
        let device = metal::Device::system_default()?;
        let queue = device.new_command_queue();
        Some(MetalAtlas::new(device, true, queue))
    }

    fn make_image_key(image_id: usize, frame_index: usize) -> AtlasKey {
        AtlasKey::Image(hgpui::RenderImageParams {
            image_id: hgpui::ImageId(image_id),
            frame_index,
        })
    }

    fn insert_tile(atlas: &MetalAtlas, key: &AtlasKey, size: Size<DevicePixels>) -> AtlasTile {
        atlas
            .get_or_insert_with(key, &mut || {
                let byte_count = (size.width.0 as usize) * (size.height.0 as usize) * 4;
                Ok(Some((size, Cow::Owned(vec![0u8; byte_count]))))
            })
            .expect("allocation should succeed")
            .expect("callback returns Some")
    }

    struct SignalOnDrop(metal::SharedEvent);

    impl Drop for SignalOnDrop {
        fn drop(&mut self) {
            self.0.set_signaled_value(1);
        }
    }

    #[test]
    fn shadow_retirement_waits_for_in_flight_reads_and_preserves_reloaded_key() {
        objc::rc::autoreleasepool(|| {
            let atlas = create_atlas().expect("Metal device should be available");
            let keys: Vec<_> = (0..3).map(|index| make_image_key(100 + index, 0)).collect();
            let size = Size { width: DevicePixels(1024), height: DevicePixels(1024) };
            let tiles: Vec<_> = keys.iter().map(|key| insert_tile(&atlas, key, size)).collect();
            let gate = SignalOnDrop(atlas.0.lock().device.new_shared_event());
            let in_flight = atlas.1.new_command_buffer();
            in_flight.encode_wait_for_event(&gate.0, 1);
            in_flight.commit();
            let retirement = atlas.retire_tiles(&keys[1..]).expect("retirement submission");
            assert!(atlas.retire_tiles(&keys[1..]).is_none(), "duplicate retirement is harmless");
            {
                let state = atlas.0.lock();
                assert_eq!(state.tiles_by_key.len(), 1);
                assert_eq!(state.tiles_by_key[&keys[0]], tiles[0]);
                assert_eq!(state.polychrome_textures.textures.iter().flatten().count(), 3,
                    "in-flight pages must not be freed or reused before the queue boundary");
                assert!(state.polychrome_textures.free_list.is_empty());
            }
            let reloaded = insert_tile(&atlas, &keys[1], size);
            assert_ne!(reloaded.texture_id, tiles[1].texture_id,
                "CPU upload must not overwrite a retired but in-flight page");
            drop(gate);
            retirement.wait_until_completed();
            let state = atlas.0.lock();
            assert_eq!(state.polychrome_textures.textures.iter().flatten().count(), 2);
            assert_eq!(state.polychrome_textures.free_list.len(), 2);
            assert_eq!(state.tiles_by_key[&keys[0]], tiles[0]);
            assert_eq!(state.tiles_by_key[&keys[1]], reloaded,
                "late completion must not revoke a reloaded instance of the same key");
        });
    }

    #[test]
    fn shadow_retirement_completes_after_atlas_owner_drop() {
        let state = objc::rc::autoreleasepool(|| {
            let atlas = create_atlas().expect("Metal device should be available");
            let key = make_image_key(100, 0);
            insert_tile(&atlas, &key, Size { width: DevicePixels(1), height: DevicePixels(1) });
            let state = std::sync::Arc::downgrade(&atlas.0);
            let gate = SignalOnDrop(atlas.0.lock().device.new_shared_event());
            let in_flight = atlas.1.new_command_buffer();
            in_flight.encode_wait_for_event(&gate.0, 1);
            in_flight.commit();
            let retirement = atlas.retire_tiles(&[key]).expect("retirement submission");
            drop(atlas);
            assert!(state.upgrade().is_some(), "callback owns resources until safe completion");
            drop(gate);
            retirement.wait_until_completed();
            state
        });
        assert!(state.upgrade().is_none(), "completion and autorelease must release final state");
    }

    #[test]
    fn atlas_rejects_empty_tiles_without_publishing_pages() {
        let atlas = create_atlas().expect("Metal device should be available");
        let key = make_image_key(1, 0);
        let small = Size {
            width: DevicePixels(1),
            height: DevicePixels(1),
        };
        let tile = insert_tile(&atlas, &key, small);
        atlas.remove(&key);

        for (width, height) in [(0, 1), (1, 0), (-1, 1), (1, -1), (16385, 1), (1, 16385)] {
            let result = atlas.get_or_insert_with(&key, &mut || {
                Ok(Some((
                    Size {
                        width: DevicePixels(width),
                        height: DevicePixels(height),
                    },
                    Cow::Borrowed(&[]),
                )))
            });
            assert!(result.is_err());
            let state = atlas.0.lock();
            assert_eq!(state.polychrome_textures.textures.len(), 1);
            assert!(state.polychrome_textures.textures[0].is_none());
            assert_eq!(state.polychrome_textures.free_list, vec![0]);
            assert!(state.tiles_by_key.is_empty());
        }

        let next = insert_tile(&atlas, &key, small);
        assert_eq!(next.texture_id, tile.texture_id);
    }

    #[test]
    fn atlas_failed_first_allocation_preserves_new_and_reused_slots() {
        let atlas = create_atlas().expect("Metal device should be available");
        for reuse_slot in [false, true] {
            if reuse_slot {
                let key = make_image_key(1, 0);
                insert_tile(
                    &atlas,
                    &key,
                    Size {
                        width: DevicePixels(1),
                        height: DevicePixels(1),
                    },
                );
                atlas.remove(&key);
            }
            let mut state = atlas.0.lock();
            for _ in 0..3 {
                // Force a first-allocation failure on a small real GPU page.
                assert!(
                    state
                        .allocate_new_texture(
                            Size {
                                width: DevicePixels(1025),
                                height: DevicePixels(1)
                            },
                            Size {
                                width: DevicePixels(1024),
                                height: DevicePixels(1024)
                            },
                            AtlasTextureKind::Polychrome,
                        )
                        .is_none()
                );
                assert_eq!(
                    state.polychrome_textures.textures.len(),
                    usize::from(reuse_slot)
                );
                assert!(state.polychrome_textures.iter_mut().next().is_none());
                assert_eq!(
                    state.polychrome_textures.free_list,
                    if reuse_slot { vec![0] } else { vec![] }
                );
            }
        }
    }

    #[test]
    fn test_remove_clears_stale_keys_from_tiles_by_key() {
        let Some(atlas) = create_atlas() else {
            return;
        };

        let small = Size {
            width: DevicePixels(64),
            height: DevicePixels(64),
        };

        let key_a = make_image_key(1, 0);
        let key_b = make_image_key(2, 0);
        let key_c = make_image_key(3, 0);

        let tile_a = insert_tile(&atlas, &key_a, small);
        let tile_b = insert_tile(&atlas, &key_b, small);
        let tile_c = insert_tile(&atlas, &key_c, small);

        assert_eq!(tile_a.texture_id, tile_b.texture_id);
        assert_eq!(tile_b.texture_id, tile_c.texture_id);

        // Remove A: texture still has B and C, so it stays.
        // The key for A must be removed from tiles_by_key.
        atlas.remove(&key_a);

        // Remove B: texture still has C.
        atlas.remove(&key_b);

        // Remove C: texture becomes unreferenced and is deleted.
        atlas.remove(&key_c);

        // Re-inserting A must allocate a fresh tile on a new texture,
        // NOT return a stale tile referencing the deleted texture.
        let tile_a2 = insert_tile(&atlas, &key_a, small);

        // The texture must actually exist — this would panic before the fix.
        let _texture = atlas.metal_texture(tile_a2.texture_id);
    }

    #[test]
    fn test_remove_deallocates_tile_space_for_reuse() {
        let Some(atlas) = create_atlas() else {
            return;
        };

        let small = Size {
            width: DevicePixels(64),
            height: DevicePixels(64),
        };
        let big = Size {
            width: DevicePixels(700),
            height: DevicePixels(700),
        };

        let keeper_key = make_image_key(1, 0);
        let big_key_a = make_image_key(2, 0);
        let big_key_b = make_image_key(3, 0);

        let keeper_tile = insert_tile(&atlas, &keeper_key, small);
        let tile_a = insert_tile(&atlas, &big_key_a, big);
        assert_eq!(keeper_tile.texture_id, tile_a.texture_id);

        atlas.remove(&big_key_a);
        let tile_b = insert_tile(&atlas, &big_key_b, big);
        assert_eq!(tile_b.texture_id, keeper_tile.texture_id);
    }

    #[test]
    fn test_remove_nonexistent_key_is_noop() {
        let Some(atlas) = create_atlas() else {
            return;
        };
        let key = make_image_key(999, 0);
        atlas.remove(&key);
    }
}
