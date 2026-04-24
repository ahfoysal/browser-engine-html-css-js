//! JS runtime + DOM bindings.
//!
//! We keep an arena of mutable DOM nodes on the Rust side. JS never holds a
//! native handle directly — it holds integer node ids and all DOM operations
//! are routed through global functions exposed into the QuickJS runtime.
//!
//! A thin JS shim (see `SHIM`) wraps raw node ids into Element-looking objects
//! that expose `textContent`, `style`, `addEventListener`, `getAttribute`, etc.
//! so page scripts can be written in idiomatic browser-style JS.

pub mod dom;
pub mod runtime;

pub use dom::{Dom, NodeId};
pub use runtime::JsRuntime;
