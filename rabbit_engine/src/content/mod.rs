//! Content model for the Rabbit protocol.
//!
//! Menus (rabbitmaps) and plain text content are registered in a
//! [`ContentStore`](store::ContentStore) and served by the
//! [`handle_list`](handler::handle_list) and
//! [`handle_fetch`](handler::handle_fetch) functions.

pub mod handler;
pub mod store;
