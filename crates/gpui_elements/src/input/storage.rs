use crate::input::unicode::UnicodeString;
use std::ops::Range;

pub trait InputStorage: UnicodeString {
    fn len(&self) -> usize;
    fn as_str(&self) -> &str;
    fn emplace(&mut self, s: &str);
    fn update_utf8(&mut self, range_utf8: Range<usize>, text: &str);
    fn replace_range(&mut self, range: Range<usize>, text: &str);
}

/// A light wrapper around std String as a storage medium for `InputState`.
#[derive(Default)]
pub struct Standard {
    inner: String,
    /// Cached UTF-16 length of content for faster IME operations. Lazily computed when queried.
    cached_utf16_len: Option<usize>,
}
impl std::ops::Deref for Standard {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl std::ops::DerefMut for Standard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
impl UnicodeString for Standard {
    fn len_utf16_cached(&self) -> Option<usize> {
        self.cached_utf16_len
    }

    fn content_utf8(&self) -> &str {
        &self.inner
    }

    fn len_utf16(&self) -> usize {
        if let Some(len) = self.cached_utf16_len {
            return len;
        }
        self.inner.chars().map(|c| c.len_utf16()).sum()
    }

    fn clear_utf16_cache(&mut self) {
        self.cached_utf16_len = None;
    }
}
impl InputStorage for Standard {
    fn len(&self) -> usize {
        self.inner.len()
    }

    fn as_str(&self) -> &str {
        self.inner.as_str()
    }

    fn emplace(&mut self, s: &str) {
        self.inner = s.to_owned();
        self.cached_utf16_len = None;
    }

    fn update_utf8(&mut self, range_utf8: Range<usize>, text: &str) {
        if let Some(cached_len) = self.cached_utf16_len {
            let removed_utf16_len: usize =
                self.inner[range_utf8].chars().map(|c| c.len_utf16()).sum();
            let added_utf16_len: usize = text.chars().map(|c| c.len_utf16()).sum();
            self.cached_utf16_len = Some(cached_len - removed_utf16_len + added_utf16_len);
        }
    }

    fn replace_range(&mut self, range: Range<usize>, text: &str) {
        self.inner.replace_range(range, &text);
    }
}
