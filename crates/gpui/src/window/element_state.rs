use std::{
    any::{Any, TypeId},
    hash::BuildHasher,
    sync::Arc,
};

use collections::FxBuildHasher;
use hashbrown::{HashMap, hash_map::RawEntryMut};

use crate::{ElementId, GlobalElementId, Window};

pub(super) type ElementStateKey = (GlobalElementId, TypeId);

pub(super) struct ElementStateBox {
    pub(super) inner: Box<dyn Any>,
    #[cfg(debug_assertions)]
    pub(super) type_name: &'static str,
}

#[derive(Default)]
pub(super) struct ElementStates(HashMap<ElementStateKey, ElementStateBox, FxBuildHasher>);

impl ElementStates {
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }

    pub(super) fn hash(path: &[ElementId], type_id: TypeId) -> u64 {
        // GlobalElementId (including its Arc) hashes as the underlying slice.
        // Keep this equivalent to hashing the owned key, including on table growth.
        FxBuildHasher.hash_one((path, type_id))
    }

    pub(super) fn take(
        &mut self,
        path: &[ElementId],
        type_id: TypeId,
        hash: u64,
    ) -> Option<(ElementStateKey, ElementStateBox)> {
        match self
            .0
            .raw_entry_mut()
            .from_hash(hash, |(id, ty)| *ty == type_id && id.0.as_ref() == path)
        {
            RawEntryMut::Occupied(entry) => Some(entry.remove_entry()),
            RawEntryMut::Vacant(_) => None,
        }
    }

    pub(super) fn insert(&mut self, key: ElementStateKey, hash: u64, state: ElementStateBox) {
        match self.0.raw_entry_mut().from_key_hashed_nocheck(hash, &key) {
            RawEntryMut::Occupied(mut entry) => {
                // The user callback ran with no entry borrowed; it could have set this key.
                entry.insert(state);
            }
            RawEntryMut::Vacant(entry) => {
                entry.insert_hashed_nocheck(hash, key, state);
            }
        }
    }

    pub(super) fn replay_from(&mut self, previous: &mut Self, accessed: &[ElementStateKey]) {
        for key in accessed {
            if previous.0.is_empty() {
                break;
            }
            if let Some((key, state)) = previous.0.remove_entry(key) {
                self.0.insert(key, state);
            }
        }
    }
}

impl Window {
    // Hooks can look up the current ID stack without snapshotting it into an Arc first.
    pub(super) fn with_element_state_inner<S: 'static, R>(
        &mut self,
        global_id: Option<&GlobalElementId>,
        f: impl FnOnce(Option<S>, &mut Self) -> (R, S),
    ) -> R {
        self.invalidator.debug_assert_paint_or_prepaint();

        let path = global_id.map_or(self.element_id_stack.as_slice(), |id| id.0.as_ref());
        let type_id = TypeId::of::<S>();
        let hash = ElementStates::hash(path, type_id);
        let existing = self
            .next_frame
            .element_states
            .take(path, type_id, hash)
            .or_else(|| self.rendered_frame.element_states.take(path, type_id, hash));
        let (key, previous) = match existing {
            Some((key, state)) => (key, Some(state)),
            None => {
                let id = global_id
                    .cloned()
                    .unwrap_or_else(|| GlobalElementId(Arc::from(path)));
                ((id, type_id), None)
            }
        };
        self.next_frame.accessed_element_states.push(key.clone());

        let (result, boxed) = if let Some(previous) = previous {
            // The extra option lets us give ownership of S to the callback without
            // allocating a new box, or holding a borrow into either frame's table.
            let mut state_box = previous
                .inner
                .downcast::<Option<S>>()
                .map_err(|_| {
                    #[cfg(debug_assertions)]
                    {
                        anyhow::anyhow!(
                            "invalid element state type for id, requested {:?}, actual: {:?}",
                            std::any::type_name::<S>(),
                            previous.type_name
                        )
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        anyhow::anyhow!(
                            "invalid element state type for id, requested {:?}",
                            std::any::type_name::<S>()
                        )
                    }
                })
                .unwrap();
            let state = state_box.take().expect(
                "reentrant call to with_element_state for the same state type and element id",
            );
            let (result, state) = f(Some(state), self);
            state_box.replace(state);
            (result, state_box)
        } else {
            let (result, state) = f(None, self);
            (result, Box::new(Some(state)))
        };
        self.next_frame.element_states.insert(
            key,
            hash,
            ElementStateBox {
                inner: boxed,
                #[cfg(debug_assertions)]
                type_name: std::any::type_name::<S>(),
            },
        );
        result
    }
}
