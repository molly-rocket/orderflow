pub mod book;
pub mod convert;
pub mod record;
pub mod subjects;

#[allow(
    clippy::all,
    clippy::pedantic,
    missing_debug_implementations,
    unreachable_pub
)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/orderflow.v1.rs"));
}
