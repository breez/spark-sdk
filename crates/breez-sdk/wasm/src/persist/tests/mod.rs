#[cfg(not(feature = "browser-tests"))]
mod node;

#[cfg(feature = "browser-tests")]
mod web;
