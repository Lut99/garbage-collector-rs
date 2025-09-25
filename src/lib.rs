//  LIB.rs
//    by Lut99
//
//  Description:
//!   A very generic, static data structure for promoting anything to 'static lifetime at the cost of
//!   making them garbage collected.
//!
//!
//!   # Functionality
//!   The use-case for this crate is to patch your code: you are somewhere deep in the weeds and have
//!   encountered an awkward situation where you needed to have a longer reference than you do.
//!   This crate contributes the `GarbageCollector` struct to workaround the problem.
//!   It is a simple data structure for storing a sequence of arbitrary objects `T` in. The idea is that
//!   you can defer (clones of) local references to it, which is defined somewhere where it outlives the
//!   current scope, and you can use its longer-lived references.
//!   The struct is fully thread-safe, meaning that you can also declare it as `'static` to make
//!   `'static` objects.
//!
//!   An example:
//!   ```rust
//!   use garbage_collector::GarbageCollector;
//!
//!   static DUMP: GarbageCollector<String> = GarbageCollector::new();
//!
//!   fn hello_world(s: String) -> &'static str {
//!       // We register `s` as a tracked object. That will return a reference with the lifetime of
//!       // `DUMP`, which, in this case, makes it `'static`!
//!       DUMP.register(s)
//!   }
//!
//!   // This now works!
//!   assert_eq!(hello_world(String::from("Hello, world!")), "Hello, world!");
//!
//!   // Upon destruction, the `String` is deallocated, which is now the end of the program.
//!   ```
//!
//!   ## Cleaning mid-lifetime
//!   The (unsafe!) `GarbageCollector::clean()`-function allows you to deallocate all tracked objects
//!   before the collector itself is deallocated.
//!
//!   This is an incredibly unsafe operation, because it requires you to guarantee that **no objects
//!   currently registered are referenced!**.
//!   If you do, it will trigger use-after-free errors, which is Undefined Behaviour (UB).
//!
//!   However, if you know what you are doing, it can help you save memory.
//!   Our example again:
//!   ```rust
//!   use garbage_collector::GarbageCollector;
//!
//!   static DUMP: GarbageCollector<String> = GarbageCollector::new();
//!
//!   fn hello_world(s: String) -> &'static str {
//!       // We register `s` as a tracked object. That will return a reference with the lifetime of
//!       // `DUMP`, which, in this case, makes it `'static`!
//!       DUMP.register(s)
//!   }
//!
//!   // This now works!
//!   assert_eq!(hello_world(String::from("Hello, world!")), "Hello, world!");
//!
//!   // Upon destruction, the `String` is deallocated, which is now the end of the program.
//!   // ...or we do it earlier, because we are sure that `Hello, world!` is never referenced anymore!
//!   unsafe { DUMP.clean() };
//!   ```
//!
//!   ## Features
//!   The crate has the following features to make your life easier:
//!   - `parking_lot`: Will use the
//!     [`Mutex`](https://docs.rs/parking_lot/latest/parking_lot/type.Mutex.html) provided by the
//!     [`parking_lot`](https://docs.rs/parking_lot)-crate instead of the one provided by `std`.
//!
//!
//!   # Usage
//!   To use this library in your own project, simply add it to your `Cargo.toml` file:
//!   ```toml
//!   [dependencies]
//!   garbage-collector = { git = "https://github.com/Lut99/garbage-collector-rs" }
//!   ```
//!
//!   You can commit to a specific version by mentioning the tag:
//!   ```toml
//!   [dependencies]
//!   garbage-collector = { git = "https://github.com/Lut99/garbage-collector-rs", tag = "v0.1.0" }
//!   ```
//!
//!   ## Generating docs
//!   To see the code documentation, run:
//!   ```sh
//!   cargo doc --open
//!   ```
//!   in the root of the crate.
//!
//!
//!   # Contributing
//!   Contributions to this crate are welcome! Please feel free to
//!   [leave an issue](https://github.com/Lut99/garbage-collector-rs/issues) or
//!   [create a pull request](https://github.com/Lut99/garbage-collector-rs/pulls).
//!
//!
//!   # License
//!   This project is licensed under the Apache 2.0 license. See [`./LICENSE`](./LICENSE) for more
//!   information.
//

use std::fmt::{Debug, Formatter, Result as FResult};
#[cfg(not(feature = "parking_lot"))]
use std::sync::{Mutex, MutexGuard};

#[cfg(feature = "parking_lot")]
use parking_lot::{Mutex, MutexGuard};


/***** HELPER MACROS *****/
/// Macro for ensuring we don't care about poisoning.
macro_rules! lock {
    ($mutex:expr) => {{
        #[cfg(not(feature = "parking_lot"))]
        {
            $mutex.lock().unwrap_or_else(|err| panic!("Poisoned internal lock: {err}"))
        }
        #[cfg(feature = "parking_lot")]
        {
            $mutex.lock()
        }
    }};
}





/***** LIBRARY *****/
/// Defines a static garbage collector for promoting anything to a `'static` lifetime by reference.
///
/// This magic is achieved at the cost of:
/// - The memory will not be cleared until either 1) the program gracefully exits or 2) it is
///   manually cleared (and the latter is unsafe!); and
/// - Access to a global lock is required to register the object for garbage collection. Hence,
///   creating the 'statics is quite expensive, potentially.
///
/// Hence, this struct is designed as a **last resort:** if you are somehow forced to return a
/// reference that needs to outlive the current context, you can fall back to this struct to fix
/// it.
pub struct GarbageCollector<T> {
    /// The list of Garbage-Collected things.
    data: Mutex<Vec<*const T>>,
}

// Markers
// SAFETY: Adding this marker is OK if `T` is `Sync`, because nobody can mutate a `T` once it's
// created and creation does not invalidate existing `T`s. Only `GarbageCollector::clean()` is
// problemetic, but that's unsafe anyway.
unsafe impl<T: Sync> Sync for GarbageCollector<T> {}

// Constructors
impl<T> GarbageCollector<T> {
    /// Constructor for the GarbageCollector.
    ///
    /// # Returns
    /// A new GarbageCollector that doesn't have any items yet.
    #[inline]
    pub const fn new() -> Self { Self { data: Mutex::new(Vec::new()) } }
}

// Destructors
impl<T> Drop for GarbageCollector<T> {
    #[inline]
    fn drop(&mut self) {
        // Simply drop everything
        for obj in lock!(self.data).drain(..) {
            // SAFETY: We can interpret the `obj_prime` as a valid reference to `T` because we are
            // the authority on whether it exists or not. Hence, we take care it is valid iff it is
            // present in `self.data`.
            // Further, by lifetime semantics we can be sure nothing `register()`ed is still alive.
            // So it's safe to drop all of this.
            drop(unsafe { Box::from_raw(obj as *mut T) })
        }
    }
}

// Ops
impl<T: Debug> Debug for GarbageCollector<T> {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> FResult {
        struct VecWrapper<'a, T>(MutexGuard<'a, Vec<*const T>>);
        impl<'a, T: Debug> Debug for VecWrapper<'a, T> {
            #[inline]
            fn fmt(&self, f: &mut Formatter<'_>) -> FResult {
                let mut fmt = f.debug_list();
                for obj in self.0.iter() {
                    // SAFETY: We can interpret the `obj_prime` as a valid reference to `T` because
                    // we are  the authority on whether it exists or not. Hence, we take care it is
                    // valid iff it is present in `self.data`.
                    fmt.entry(unsafe { (*obj).as_ref().unwrap_unchecked() });
                }
                fmt.finish()
            }
        }

        // Debug ourselves now and use the newtype to have the list formatter implement `Debug`
        f.debug_struct("GarbageCollector").field("data", &VecWrapper(lock!(self.data))).finish()
    }
}

// Garbage collecting
impl<T> GarbageCollector<T> {
    /// Register an object for management by the GarbageCollector.
    ///
    /// Note that this function is relatively expensive due to a struct-wide lock. Use as last
    /// resort only!
    ///
    /// # Arguments
    /// - `obj`: The object to register.
    ///
    /// # Returns
    /// A reference with the lifetime of the collector to the given `obj`ect. This object will be
    /// valid until the end of the program, **or until you call [`GarbageCollector::clean()`].**
    /// See it for more information.
    #[inline]
    #[track_caller]
    pub fn register(&self, obj: T) -> &T {
        // First, put the object on the heap and get a pointer to it
        let obj: *const T = Box::into_raw(Box::new(obj));

        // Then, register the object for tracking and deallocation.
        lock!(self.data).push(obj);

        // Now return a reference to it.
        // SAFETY: This is allowed because there is no (safe!) way for the user to (re)move the
        // value. Hence, as long as we exist (and therefore the memory exists), the user can safely
        // access `T`.
        unsafe { obj.as_ref().unwrap_unchecked() }
    }
}
impl<T: PartialEq> GarbageCollector<T> {
    /// Register an object for management by the GarbageCollector.
    ///
    /// This function is more memory efficient than [`GarbageCollector::register()`] because it
    /// will only allocate the object if it's not already registered. The latter happens when an
    /// object has been registered for which [`T::eq()`](PartialEq::eq()) returns **true**.
    ///
    /// # Arguments
    /// - `obj`: The object to register.
    ///
    /// # Returns
    /// A reference with the lifetime of the collector to the given `obj`ect. If the object was
    /// already present, then a lifetime to **that** object is returned instead (and `obj` is
    /// [dropped](Drop::drop())).
    ///
    /// A reference with the lifetime of the collector to the given `obj`ect. This object will be
    /// valid until the end of the program, **or until you call [`GarbageCollector::clean()`].**
    /// See it for more information.
    #[inline]
    #[track_caller]
    pub fn register_dedup(&self, obj: T) -> &T {
        // First, check if the object already exists
        {
            let data = lock!(self.data);
            for obj_prime in data.iter() {
                // SAFETY: We can interpret the `obj_prime` as a valid reference to `T` because we are
                // the authority on whether it exists or not. Hence, we take care it is valid iff it is
                // present in `self.data`.
                let obj_prime: &T = unsafe { (*obj_prime).as_ref().unwrap_unchecked() };
                if &obj == obj_prime {
                    return obj_prime;
                }
            }
        }

        // Else, register it as usual
        self.register(obj)
    }
}
impl<T> GarbageCollector<T> {
    /// Cleans all objects tracked by the GarbageCollector.
    ///
    /// # Safety
    /// This function is only safe to call if **no references returned by
    /// [`GarbageCollector::register()`] or [`GarbageCollector::register_dedup()`] exist!** _(Also
    /// not across threads!!!)_ This because the returned objects will be cleared.
    ///
    /// The safe equivalent to this action is to drop the collector as a whole. Lifetime semantics
    /// will make sure that this is a safe operation to do.
    #[inline]
    pub unsafe fn clean(&self) {
        // Simply drop everything
        for obj in lock!(self.data).drain(..) {
            // SAFETY: We can interpret the `obj_prime` as a valid reference to `T` because we are
            // the authority on whether it exists or not. Hence, we take care it is valid iff it is
            // present in `self.data`.
            // Further, we have now deferred the responsibility of not having any references around
            // to `obj` to the user.
            drop(unsafe { Box::from_raw(obj as *mut T) })
        }
    }
}
