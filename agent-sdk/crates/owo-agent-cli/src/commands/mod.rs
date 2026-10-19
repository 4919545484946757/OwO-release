// §12.3 CLI 拆分：命令域模块（每域一文件，自 main.rs 逐批机械外移）。

pub(crate) mod audit;
pub(crate) mod backup;
pub(crate) mod bench;
pub(crate) mod capabilities;
pub(crate) mod cloud;
pub(crate) mod daemon;
pub(crate) mod doctor;
pub(crate) mod eval;
pub(crate) mod mcp;
pub(crate) mod plugins;
pub(crate) mod repl;
pub(crate) mod repl_daemon;
pub(crate) mod review;
pub(crate) mod serve;
pub(crate) mod sessions;
pub(crate) mod symbols;
pub(crate) mod trust;
pub(crate) mod turn;
pub(crate) mod worker;
pub(crate) mod workflow;
