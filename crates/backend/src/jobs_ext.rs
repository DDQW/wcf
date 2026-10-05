//! Background jobs of modules added in later phases.

use crate::state::AppState;

pub async fn every_minute(_state: &AppState) {}

pub async fn hourly(_state: &AppState) {}
