mod agent_allowance;
mod dispatch;
mod escrow;
mod htlc_payment;
mod serialize;
mod subscription;
mod vault;

pub(crate) use dispatch::cmd_template;

#[cfg(test)]
mod tests;
