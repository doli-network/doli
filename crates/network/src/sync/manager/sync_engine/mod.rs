//! Sync engine — orchestrates header-first sync, body download, and request/response handling

mod body_auth;
mod decision;
mod dispatch;
mod response;

pub(in crate::sync::manager) use dispatch::PARK_EMPTY_HEADERS;
