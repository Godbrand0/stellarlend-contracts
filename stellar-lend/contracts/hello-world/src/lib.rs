#![no_std]
use soroban_sdk::{contract, contractimpl, Env};

pub mod admin;
pub mod user;
pub mod pool;
pub mod views;
pub mod deposit;
pub mod borrow;
pub mod repay;
pub mod withdraw;
pub mod reserve;

#[contract]
pub struct HelloContract;

#[contractimpl]
impl HelloContract {
    pub fn hello() {}
}

#[cfg(test)]
mod withdraw_after_repay_test;
