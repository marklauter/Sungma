//! A list with at least one item, so code that needs one can't be handed
//! none.

use std::{num::NonZeroUsize, ops::Deref, slice, vec};

/// A list with at least one item. It derefs to a slice, so it reads like
/// one, and every way to build it keeps at least one item.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NonEmpty<T>(Vec<T>);

/// `nonempty![a, b]` builds a [`NonEmpty`] from at least one item, so an
/// empty list doesn't compile. `nonempty![item; n]` repeats an item `n`
/// times, where `n` is a constant, so a count of 0 doesn't compile either:
///
/// ```compile_fail
/// let none = sungma::nonempty![1; 0];
/// ```
#[macro_export]
macro_rules! nonempty {
    ($item:expr; $count:expr) => {
        $crate::non_empty::NonEmpty::repeat(
            $item,
            const { ::std::num::NonZeroUsize::new($count).expect("repeated at least once") },
        )
    };
    ($first:expr $(, $rest:expr)* $(,)?) => {
        $crate::non_empty::NonEmpty::with_rest($first, vec![$($rest),*])
    };
}

impl<T> NonEmpty<T> {
    /// `None` when `items` is empty.
    pub fn new(items: Vec<T>) -> Option<Self> {
        (!items.is_empty()).then_some(Self(items))
    }

    pub fn one(item: T) -> Self {
        Self(vec![item])
    }

    /// `item`, `count` times.
    pub fn repeat(item: T, count: NonZeroUsize) -> Self
    where
        T: Clone,
    {
        Self(vec![item; count.get()])
    }

    /// `first` followed by `rest`.
    pub fn with_rest(first: T, rest: Vec<T>) -> Self {
        let mut items = Vec::with_capacity(rest.len() + 1);
        items.push(first);
        items.extend(rest);
        Self(items)
    }

    /// The first item, and the items after it.
    pub fn first_and_rest(&self) -> (&T, &[T]) {
        let (first, rest) = self.0.split_at(1);
        (&first[0], rest)
    }

    /// Adds `other`'s items after these.
    pub fn append(&mut self, other: Self) {
        self.0.extend(other.0);
    }

    pub fn map<U>(self, f: impl FnMut(T) -> U) -> NonEmpty<U> {
        NonEmpty(self.0.into_iter().map(f).collect())
    }

    /// Like [`NonEmpty::map`], stopping at the first error.
    pub fn try_map<U, E>(self, f: impl FnMut(T) -> Result<U, E>) -> Result<NonEmpty<U>, E> {
        Ok(NonEmpty(
            self.0.into_iter().map(f).collect::<Result<_, _>>()?,
        ))
    }
}

impl<T> Deref for NonEmpty<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T> IntoIterator for NonEmpty<T> {
    type Item = T;
    type IntoIter = vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a NonEmpty<T> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
