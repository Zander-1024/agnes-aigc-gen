mod app;
pub mod chat;

use anyhow::Result;

pub fn run() -> Result<()> {
    app::run()
}
