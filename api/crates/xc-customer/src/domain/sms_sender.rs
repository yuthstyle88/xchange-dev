use std::future::Future;
use std::pin::Pin;

use pos_error::AppResult;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait SmsSender: Send + Sync {
    fn send<'a>(&'a self, phone: &'a str, text: &'a str) -> BoxFuture<'a, AppResult<()>>;
}
