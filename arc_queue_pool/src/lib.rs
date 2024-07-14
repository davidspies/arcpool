use std::cmp::Ordering;
use std::fmt::{self, Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Deref;

use consume_on_drop::ConsumeOnDrop;
use derive_where::derive_where;

use self::pool::ArcInner;

pub use self::pool::ArcPool;

mod pool;

#[derive_where(Clone)]
pub struct Arc<T>(ConsumeOnDrop<ArcInner<T>>);

impl<T> ArcPool<T> {
    pub fn alloc(&self, value: T) -> Arc<T> {
        Arc(ConsumeOnDrop::new(self.alloc_inner(value)))
    }
}

impl<T> Arc<T> {
    /// Returns the next Arc created after this one, if it exists.
    pub fn next(this: &Self) -> Option<Self> {
        this.0.next().map(|inner| Self(ConsumeOnDrop::new(inner)))
    }

    /// If the refcount is 1 and all prior Arcs have been dropped, consumes the Arc
    /// returning its value along with the next Arc created after this one, if it exists.
    /// 
    /// If this method is called concurrently on every clone of the earliest Arc in the pool,
    /// it is guaranteed that exactly one of them will return `Some(_)`. Furthermore, no other
    /// Arc will be silently dropped.
    pub fn into_inner_and_next(this: Self) -> Option<(T, Option<Self>)> {
        let (value, next) = ConsumeOnDrop::into_inner(this.0).into_inner_and_next()?;
        Some((value, next.map(|inner| Self(ConsumeOnDrop::new(inner)))))
    }

    /// If the refcount is 1 and all prior Arcs have been dropped, consumes the Arc
    /// returning its value along with the next Arc created after this one, if it exists.
    /// On failure, returns the argument.
    pub fn try_unwrap_and_next(this: Self) -> Result<(T, Option<Self>), Self> {
        match ConsumeOnDrop::into_inner(this.0).try_unwrap_and_next() {
            Ok((value, next)) => Ok((value, next.map(|inner| Self(ConsumeOnDrop::new(inner))))),
            Err(inner) => Err(Self(ConsumeOnDrop::new(inner))),
        }
    }

    pub fn ref_count(this: &Self) -> usize {
        this.0.ref_count()
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.get()
    }
}

impl<T: Debug> Debug for Arc<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: Hash> Hash for Arc<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}

impl<T: PartialEq> PartialEq for Arc<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Eq> Eq for Arc<T> {}

impl<T: PartialOrd> PartialOrd for Arc<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (**self).partial_cmp(&**other)
    }
}

impl<T: Ord> Ord for Arc<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        (**self).cmp(&**other)
    }
}
