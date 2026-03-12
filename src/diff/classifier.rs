use crate::ast::symbol::{Parameter, Symbol};
use crate::diff::change::{ChangeKind, SignatureDelta};

/// Classify the change between two matched symbols
pub fn classify(old: &Symbol, new: &Symbol) -> ChangeKind {
    let name_changed = old.name != new.name;
    let file_changed = old.file_path != new.file_path;
    let body_changed = old.body_hash != new.body_hash;
    let vis_changed = old.visibility != new.visibility;
    let sig_changed = old.signature_differs(new);

    // Priority: Move > Rename > Signature > Body > Visibility
    if file_changed && body_changed {
        return ChangeKind::MovedAndModified {
            from_file: old.file_path.clone(),
            to_file: new.file_path.clone(),
        };
    }

    if file_changed {
        return ChangeKind::Moved {
            from_file: old.file_path.clone(),
            to_file: new.file_path.clone(),
        };
    }

    if name_changed {
        return ChangeKind::Renamed {
            old_name: old.name.clone(),
            new_name: new.name.clone(),
        };
    }

    if sig_changed {
        return ChangeKind::SignatureChanged {
            details: compute_signature_delta(old, new),
        };
    }

    if vis_changed {
        // Visibility changed — even if body also changed slightly (e.g., due to
        // export keyword inclusion), prefer VisibilityChanged for clarity.
        // If body also has significant changes, they'll show in the diff view.
        return ChangeKind::VisibilityChanged {
            old: old.visibility.clone(),
            new: new.visibility.clone(),
        };
    }

    if body_changed {
        return ChangeKind::BodyChanged;
    }

    // No detectable change (shouldn't reach here if filtering works)
    ChangeKind::BodyChanged
}

fn compute_signature_delta(old: &Symbol, new: &Symbol) -> SignatureDelta {
    let old_param_names: std::collections::HashSet<String> =
        old.parameters.iter().map(|p| p.name.clone()).collect();
    let new_param_names: std::collections::HashSet<String> =
        new.parameters.iter().map(|p| p.name.clone()).collect();

    let params_added: Vec<Parameter> = new
        .parameters
        .iter()
        .filter(|p| !old_param_names.contains(&p.name))
        .cloned()
        .collect();

    let params_removed: Vec<Parameter> = old
        .parameters
        .iter()
        .filter(|p| !new_param_names.contains(&p.name))
        .cloned()
        .collect();

    let common_params: Vec<&str> = old
        .parameters
        .iter()
        .filter(|p| new_param_names.contains(&p.name))
        .map(|p| p.name.as_str())
        .collect();

    let params_reordered = if common_params.len() >= 2 {
        let old_order: Vec<usize> = common_params
            .iter()
            .map(|name| {
                old.parameters
                    .iter()
                    .position(|p| p.name == *name)
                    .unwrap()
            })
            .collect();
        let new_order: Vec<usize> = common_params
            .iter()
            .map(|name| {
                new.parameters
                    .iter()
                    .position(|p| p.name == *name)
                    .unwrap()
            })
            .collect();
        old_order != new_order
    } else {
        false
    };

    let return_type_changed = old.return_type != new.return_type;

    SignatureDelta {
        params_added,
        params_removed,
        params_reordered,
        return_type_changed,
        old_return_type: old.return_type.clone(),
        new_return_type: new.return_type.clone(),
    }
}
