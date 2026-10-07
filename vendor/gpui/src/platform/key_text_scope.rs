/// Native dispatch scopes distinguish unchanged printable fallback from an IME
/// commit. Invalidating every enclosing scope prevents nested composition from
/// rearming the outer key when nested dispatch returns.
#[derive(Default)]
pub(super) struct KeyTextScope {
    scopes: Vec<(Option<String>, bool)>,
}

impl KeyTextScope {
    pub(super) fn enter(&mut self) {
        self.scopes.push((None, false));
    }

    pub(super) fn arm(&mut self, text: Option<String>) {
        if let Some((scope, invalidated)) = self.scopes.last_mut()
            && !*invalidated
        {
            *scope = text;
        }
    }

    pub(super) fn exit(&mut self) {
        self.scopes.pop();
    }

    pub(super) fn invalidate(&mut self) {
        for scope in &mut self.scopes {
            *scope = (None, true);
        }
    }

    pub(super) fn is_fallback(&mut self, text: &str, replaces_range: bool) -> bool {
        let matches = !replaces_range
            && self.scopes.last().and_then(|(text, _)| text.as_deref()) == Some(text);
        if !matches {
            self.invalidate();
        }
        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_independent_commits_are_native_after_return() {
        let mut scope = KeyTextScope::default();
        scope.enter();
        scope.arm(Some("a".into()));
        assert!(scope.is_fallback("a", false));
        scope.exit();
        assert!(!scope.is_fallback("a", false));
    }

    #[test]
    fn native_first_and_transformed_insertions_are_native() {
        let mut scope = KeyTextScope::default();
        scope.enter();
        assert!(!scope.is_fallback("a", false));
        scope.arm(Some("a".into()));
        assert!(!scope.is_fallback("愛", false));
        assert!(!scope.is_fallback("a", false));
        scope.exit();
    }

    #[test]
    fn nested_marks_and_transformations_invalidate_enclosing_scope() {
        for marked in [true, false] {
            let mut scope = KeyTextScope::default();
            scope.enter();
            scope.arm(Some("a".into()));
            scope.enter();
            scope.arm(Some("b".into()));
            if marked {
                scope.invalidate();
            } else {
                assert!(!scope.is_fallback("日本", false));
            }
            scope.exit();
            assert!(!scope.is_fallback("a", false));
            scope.exit();
        }
    }

    #[test]
    fn callback_marking_cannot_be_rearmed_after_app_delivery() {
        let mut scope = KeyTextScope::default();
        scope.enter();
        scope.invalidate();
        scope.arm(Some("a".into()));
        assert!(!scope.is_fallback("a", false));
        scope.exit();
    }

    #[test]
    fn nested_ordinary_dispatch_restores_outer_scope() {
        let mut scope = KeyTextScope::default();
        scope.enter();
        scope.arm(Some("a".into()));
        scope.enter();
        scope.arm(Some("b".into()));
        assert!(scope.is_fallback("b", false));
        scope.exit();
        assert!(scope.is_fallback("a", false));
        assert!(!scope.is_fallback("a", true));
        scope.exit();
    }
}
