use super::*;

#[path = "candidates.rs"]
mod candidates;
#[path = "forms.rs"]
mod forms;
#[path = "runtime.rs"]
mod runtime;
#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
#[path = "tests/mod.rs"]
mod tests;
