//! Compliment polls (ほめ投票).
//!
//! | module | what lives there |
//! |---|---|
//! | [`bank`] | the question bank: the built-in questions shipped as data, and staff's additions |
//! | [`draw`] | drawing the four friends of a poll fairly (pure logic) |
//! | [`rounds`] | rounds and polls: starting, voting, skipping, shuffling, the reward |
//! | [`flames`] | what a user received: the inbox, the numbers, the weekly ranking |
//! | [`hints`] | the hints a receiver can buy about the sender of a flame |
pub(crate) mod bank;
pub(crate) mod draw;
pub(crate) mod flames;
pub(crate) mod hints;
pub(crate) mod rounds;
