//! Zeroizing secret container for credentials and other sensitive material.

use zeroize::Zeroize;

/// Holds sensitive data that must be cleared from memory on drop and must never
/// leak through [`Clone`], [`std::fmt::Display`], or [`std::fmt::Debug`].
///
/// [`Secret`] intentionally does **not** implement [`std::fmt::Debug`], so
/// `format!("{:?}", secret)` **does not compile** and accidental logging of
/// secrets via `{:?}` is rejected by the type checker.
///
/// Access the payload only through [`Secret::expose_secret`]. This type does
/// **not** implement [`Clone`].
///
/// `T` must implement [`Zeroize`] so bytes are overwritten when the [`Secret`]
/// is dropped.
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    /// Wraps `value`, which will be zeroized when this [`Secret`] is dropped.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Returns a reference to the inner secret. Call sites should minimize
    /// the lifetime of borrows and avoid logging or formatting the result.
    pub fn expose_secret(&self) -> &T {
        &self.0
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time guarantee: [`Secret`] must not implement [`std::fmt::Debug`].
    #[test]
    fn secret_does_not_implement_debug() {
        static_assertions::assert_not_impl_any!(Secret<String>: std::fmt::Debug);
        static_assertions::assert_not_impl_any!(Secret<Vec<u8>>: std::fmt::Debug);
    }

    #[test]
    fn expose_secret_returns_payload() {
        let secret = Secret::new("payload".to_string());
        assert_eq!(secret.expose_secret(), "payload");
    }
}
