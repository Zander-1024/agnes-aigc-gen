pub mod chat;
mod dashboard;

use anyhow::Result;

pub fn run() -> Result<()> {
    dashboard::run()
}
