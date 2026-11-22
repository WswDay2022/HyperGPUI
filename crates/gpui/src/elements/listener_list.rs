//! Sparse callback storage: no payload for empty kinds, one inline callback in the first
//! allocation, and a spill buffer only when that kind actually has multiple listeners.

use smallvec::{IntoIter, SmallVec};
use std::ops::{Deref, DerefMut, RangeFull};

pub(crate) struct ListenerList<T>(Option<Box<SmallVec<[T; 1]>>>);

impl<T> Default for ListenerList<T> {
    fn default() -> Self { Self(None) }
}

impl<T> ListenerList<T> {
    pub(crate) fn push(&mut self, listener: T) {
        self.0.get_or_insert_with(|| Box::new(SmallVec::new())).push(listener);
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.0.as_ref().map_or(0, |listeners| listeners.capacity())
    }

    pub(crate) fn take(&mut self) -> SmallVec<[T; 1]> {
        self.0.take().map(|listeners| *listeners).unwrap_or_default()
    }

    // Callers only drain the entire list. An owned iterator both preserves order/drop behaviour
    // and releases the container header immediately instead of retaining it until frame teardown.
    pub(crate) fn drain(&mut self, _: RangeFull) -> IntoIter<[T; 1]> {
        self.take().into_iter()
    }
}

impl<T> Deref for ListenerList<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.0.as_ref().map_or(&[], |listeners| listeners.as_slice())
    }
}

impl<T> DerefMut for ListenerList<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.0.as_mut().map_or(&mut [], |listeners| listeners.as_mut_slice())
    }
}

impl<T: Clone> Clone for ListenerList<T> {
    fn clone(&self) -> Self { Self(self.0.clone()) }
}

impl<T> IntoIterator for ListenerList<T> {
    type Item = T;
    type IntoIter = IntoIter<[T; 1]>;
    fn into_iter(mut self) -> Self::IntoIter { self.take().into_iter() }
}

impl<'a, T> IntoIterator for &'a ListenerList<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

#[cfg(test)]
mod tests {
    use super::ListenerList;
    use crate::elements::test_allocations::measure;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn empty_and_single_storage_have_bounded_requests() {
        let (mut list, counts) = measure(ListenerList::<u64>::default);
        assert_eq!(counts.count, 0);
        assert!(list.is_empty());
        assert_eq!(std::mem::size_of_val(&list), std::mem::size_of::<usize>());
        let (_, counts) = measure(|| list.push(7));
        assert_eq!(counts.count, 1);
        assert_eq!(list.capacity(), 1);
        assert_eq!(&*list, &[7]);
    }

    #[test]
    fn spilling_cloning_and_draining_preserve_order() {
        let mut list = ListenerList::default();
        for value in 0..20 { list.push(value); }
        assert_eq!(list.iter().copied().collect::<Vec<_>>(), (0..20).collect::<Vec<_>>());
        let cloned = list.clone();
        assert_eq!(cloned.into_iter().collect::<Vec<_>>(), (0..20).collect::<Vec<_>>());
        assert_eq!(list.drain(..).collect::<Vec<_>>(), (0..20).collect::<Vec<_>>());
        assert!(list.is_empty());
        assert_eq!(list.capacity(), 0);
    }

    #[test]
    fn take_single_can_be_cloned_without_a_second_storage_allocation() {
        let mut list = ListenerList::default();
        list.push(Rc::new(Cell::new(7)));
        let values = list.take();
        let (cloned, counts) = measure(|| values.clone());
        assert_eq!(counts.count, 0);
        cloned[0].set(8);
        assert_eq!(values[0].get(), 8);
        assert!(list.is_empty());
    }

    #[test]
    fn dropping_a_partly_consumed_drain_drops_each_remaining_value_once() {
        struct Tracked(Rc<Cell<usize>>);
        impl Drop for Tracked { fn drop(&mut self) { self.0.set(self.0.get() + 1); } }
        let dropped = Rc::new(Cell::new(0));
        let mut list = ListenerList::default();
        for _ in 0..4 { list.push(Tracked(dropped.clone())); }
        let mut drained = list.drain(..);
        drop(drained.next());
        assert_eq!(dropped.get(), 1);
        drop(drained);
        assert_eq!(dropped.get(), 4);
        assert!(list.is_empty());
    }
}
