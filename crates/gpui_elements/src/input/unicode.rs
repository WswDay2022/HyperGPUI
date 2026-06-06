use std::ops::Range;

pub trait UnicodeString {
    fn len_utf16_cached(&self) -> Option<usize>;
    /// Returns a reference to the utf8 string.
    fn content_utf8(&self) -> &str;
    /// Returns the UTF-16 length of the content.
    fn len_utf16(&self) -> usize;

    fn utf_offset_8to16(&self, pos_uft8: usize) -> usize {
        // Fast path: if offset is 0, return 0
        if pos_uft8 == 0 {
            return 0;
        }

        // Fast path: if offset is at or past end, return cached length
        if pos_uft8 >= self.content_utf8().len() {
            return self.len_utf16();
        }

        let mut pos_utf16 = 0;
        let mut counter_utf8 = 0;

        for character in self.content_utf8().chars() {
            if counter_utf8 >= pos_uft8 {
                break;
            }
            counter_utf8 += character.len_utf8();
            pos_utf16 += character.len_utf16();
        }

        pos_utf16
    }

    fn utf_offset_16to8(&self, pos_utf16: usize) -> usize {
        // Fast path: if offset is 0, return 0
        if pos_utf16 == 0 {
            return 0;
        }

        // Fast path: if we have cached length and offset is at or past end
        if let Some(utf16_len) = self.len_utf16_cached() {
            if pos_utf16 >= utf16_len {
                return self.content_utf8().len();
            }
        }

        let mut pos_utf8 = 0;
        let mut counter_utf16 = 0;

        for character in self.content_utf8().chars() {
            if counter_utf16 >= pos_utf16 {
                break;
            }
            counter_utf16 += character.len_utf16();
            pos_utf8 += character.len_utf8();
        }

        pos_utf8.min(self.content_utf8().len())
    }

    fn utf_range_8to16(&self, range_utf8: &Range<usize>) -> Range<usize> {
        self.utf_offset_8to16(range_utf8.start)..self.utf_offset_8to16(range_utf8.end)
    }

    fn utf_range_16to8(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.utf_offset_16to8(range_utf16.start)..self.utf_offset_16to8(range_utf16.end)
    }
}
