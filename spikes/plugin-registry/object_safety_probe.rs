// PROTOTYPE (throwaway). Not product code.
//
// Standalone probe, not a workspace member and not built by the host: this file
// exists only to capture verbatim which trait shapes rustc refuses to make
// object safe. Run it with:
//
//   devenv shell -- rustc --edition 2021 --crate-type lib \
//     /tmp/metis-plugin-registry-prototype/object_safety_probe.rs -o /tmp/probe.rlib
//
// Every module below is a shape the landed `authoring::Plugin` trait uses, or a
// shape that looks like it should be usable behind `dyn`.

use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// 1. The landed trait's vocabulary: an associated const per capability arm.
pub mod associated_const {
    pub trait HasAssocConst {
        const ANSWER: &'static str = "";
        fn capabilities(&self) -> Vec<String>;
    }
    pub fn takes_dyn(_p: Box<dyn HasAssocConst>) {}
}

// 2. The landed trait's vocabulary: an associated type per capability arm.
pub mod associated_type {
    pub trait HasAssocType {
        type Answer;
        fn answer(&self);
    }
    pub fn takes_dyn(_p: Box<dyn HasAssocType>) {}
}

// 3. The landed trait's method shape: RPITIT.
pub mod rpitit {
    use std::future::Future;
    pub trait ReturnsImplFuture {
        fn answer(&self) -> impl Future<Output = ()> + Send;
    }
    pub fn takes_dyn(_p: Box<dyn ReturnsImplFuture>) {}
}

// 4. `async fn` in a trait, which is sugar for RPITIT.
pub mod async_fn {
    pub trait UsesAsyncFn {
        async fn answer(&self);
    }
    pub fn takes_dyn(_p: Box<dyn UsesAsyncFn>) {}
}

// 5. A generic method: the shape a typed `invoke<T>` would take.
pub mod generic_method {
    pub trait GenericMethod {
        fn invoke<T: Clone>(&self, value: T);
    }
    pub fn takes_dyn(_p: Box<dyn GenericMethod>) {}
}

// 6. The shape the prototype's seam uses: a boxed future with an elided
//    lifetime. This is the control, and it must compile.
pub mod boxed_future {
    use super::BoxFuture;
    pub trait Erased {
        fn invoke<'a>(&'a self, capability: &'a str) -> BoxFuture<'a, Result<Vec<u8>, ()>>;
        fn cancel(&self, request_id: u64) -> Result<(), ()>;
    }
    pub fn takes_dyn(_p: Box<dyn Erased>) {}
}

// 7. A `where Self: Sized` method: accepted on an object safe trait, but not
//    callable through the trait object.
pub mod sized_method {
    pub trait WithSizedMethod {
        fn object_safe(&self);
        fn needs_size(self) -> usize
        where
            Self: Sized,
        {
            std::mem::size_of::<Self>()
        }
    }
    pub fn calls_through_dyn(p: &dyn WithSizedMethod) {
        p.object_safe();
        // p.needs_size(); // uncomment to see the call-site error
    }
}
