//! Orders: a small module that uses each construct the Rust queries read.
use std::collections::HashMap;
use std::io::{self, Read as R};
use crate::models::*;
use serde;
extern crate log as logging;

pub const MAX_ITEMS: usize = 50;
static GREETING: &str = "hi";

pub type Id = u64;

#[derive(Debug)]
pub struct Order {
    items: Vec<Item>,
    total: Money,
}

pub enum Status {
    Open,
    Paid(Receipt),
}

pub trait Priced {
    fn price(&self) -> Money;
    fn label(&self) -> String {
        format!("{}", self.price())
    }
}

impl Order {
    pub const LIMIT: u32 = 3;

    pub fn new(items: Vec<Item>) -> Self {
        let total = sum(&items);
        Order { items, total }
    }
}

impl Priced for Order {
    fn price(&self) -> Money {
        self.total.clone()
    }
}

macro_rules! money {
    ($v:expr) => { Money::from($v) };
}

pub fn sum(items: &[Item]) -> Money {
    items.iter().map(Item::cost).fold(money!(0), add)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper() -> Order {
        Order::new(Vec::new())
    }

    #[test]
    #[should_panic]
    fn rejects_empty() {
        helper().price();
    }
}
