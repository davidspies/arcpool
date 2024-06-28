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
    pub fn next(this: &Self) -> Option<Self> {
        this.0.next().map(|inner| Self(ConsumeOnDrop::new(inner)))
    }

    pub fn into_inner_and_next(this: Self) -> Option<(T, Option<Self>)> {
        let (value, next) = ConsumeOnDrop::into_inner(this.0).into_inner_and_next()?;
        Some((value, next.map(|inner| Self(ConsumeOnDrop::new(inner)))))
    }

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

unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}
