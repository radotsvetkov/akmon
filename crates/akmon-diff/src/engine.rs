use std::marker::PhantomData;

/// Diff engine skeleton.
///
/// Item 6.1 defines type shape only. Engine execution methods are introduced in
/// Item 6.2.
#[derive(Debug, Default)]
pub struct DiffEngine<S, G> {
    _phantom: PhantomData<(S, G)>,
}

impl<S, G> DiffEngine<S, G> {
    /// Creates a new diff engine placeholder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DiffEngine;

    #[test]
    fn t_new_engine_constructs() {
        let _engine: DiffEngine<(), ()> = DiffEngine::new();
    }
}
