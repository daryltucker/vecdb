//! Hand each call and string literal to its innermost scope.
//!
//! Extraction is per language (`ts_refs` for tree-sitter, `python::refs`);
//! ownership is one rule for all of them, by byte span: an item belongs to the
//! deepest scope element whose span contains its first byte, and to the module
//! when no scope does. Spans, not names or lines, so two scopes on one line —
//! or a lambda inside a call — cannot be confused.

use crate::types::{DocumentElement, ElementType, Scope};

/// The element types that are scopes: the things a reader names when asked
/// "where does this happen?". A variable or constant is not — `conn =
/// sqlite3.connect(..)` at top level is the module's call.
pub(crate) fn is_scope(t: ElementType) -> bool {
    matches!(
        t,
        ElementType::Function
            | ElementType::Class
            | ElementType::Struct
            | ElementType::Enum
            | ElementType::Union
            | ElementType::Interface
            | ElementType::Trait
            | ElementType::Implementation
            | ElementType::Module
            | ElementType::Namespace
            | ElementType::Kernel
            | ElementType::DeviceFunction
    )
}

/// Give every scope element of `elements` (at any depth) a [`Scope`] holding
/// what lies directly inside it, and return the module scope: everything no
/// scope contains. Elements must carry their spans (`source`).
pub(crate) fn assign(elements: &mut [DocumentElement], found: Scope) -> Scope {
    mark(elements);
    let mut module = Scope::default();
    for call in found.calls {
        match innermost(elements, call.start) {
            Some(scope) => scope.calls.push(call),
            None => module.calls.push(call),
        }
    }
    for string in found.strings {
        match innermost(elements, string.start) {
            Some(scope) => scope.strings.push(string),
            None => module.strings.push(string),
        }
    }
    module
}

fn mark(elements: &mut [DocumentElement]) {
    for e in elements {
        if is_scope(e.element_type) {
            e.scope.get_or_insert_with(Scope::default);
        }
        mark(&mut e.children);
    }
}

fn innermost(elements: &mut [DocumentElement], at: usize) -> Option<&mut Scope> {
    for e in elements.iter_mut() {
        let Some((start, end)) = e.source_span() else {
            continue;
        };
        if !(start <= at && at < end) {
            continue;
        }
        // Decide first whether a child scope claims it, so the mutable borrow
        // goes to exactly one place.
        let child_claims = contains_scope(&e.children, at);
        if child_claims {
            return innermost(&mut e.children, at);
        }
        if e.scope.is_some() {
            return e.scope.as_mut();
        }
    }
    None
}

fn contains_scope(elements: &[DocumentElement], at: usize) -> bool {
    elements.iter().any(|e| {
        e.source_span()
            .is_some_and(|(start, end)| start <= at && at < end)
            && (e.scope.is_some() || contains_scope(&e.children, at))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Call, StringKind, StringLiteral};

    fn el(t: ElementType, start: usize, end: usize) -> DocumentElement {
        let mut e = DocumentElement::new(t, Some(format!("{t}@{start}")), String::new(), 1, 1);
        e.set_source_span(start, end);
        e
    }

    fn call(at: usize) -> Call {
        Call {
            callee: format!("c{at}"),
            line: 1,
            line_end: 1,
            start: at,
            end: at + 1,
            args: vec![],
        }
    }

    #[test]
    fn deepest_scope_wins_and_the_rest_is_the_module() {
        // class 10..50 { method 20..30 }, a variable 60..70 (not a scope).
        let class =
            el(ElementType::Class, 10, 50).with_children(vec![el(ElementType::Function, 20, 30)]);
        let mut elements = vec![class, el(ElementType::Variable, 60, 70)];
        let found = Scope {
            calls: vec![call(5), call(15), call(25), call(65)],
            strings: vec![StringLiteral {
                kind: StringKind::Plain,
                value: "s".into(),
                line: 1,
                line_end: 1,
                start: 22,
                end: 24,
                folded: false,
                holes: vec![],
            }],
        };
        let module = assign(&mut elements, found);
        let names = |s: &Scope| s.calls.iter().map(|c| c.callee.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(&module),
            ["c5", "c65"],
            "outside every scope, or in a non-scope"
        );
        let class_scope = elements[0].scope.as_ref().unwrap();
        assert_eq!(names(class_scope), ["c15"]);
        let method_scope = elements[0].children[0].scope.as_ref().unwrap();
        assert_eq!(names(method_scope), ["c25"]);
        assert_eq!(method_scope.strings.len(), 1);
        assert!(elements[1].scope.is_none(), "a variable is not a scope");
    }
}
