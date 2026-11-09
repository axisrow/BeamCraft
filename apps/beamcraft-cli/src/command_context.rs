// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Identical identity-option parsing shared by domain command adapters.

use beamcraft_host::revision::Uuid7;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
use session::Refusal;
use std::collections::BTreeMap;

pub(crate) const CONTEXT_OPTIONS: [&str; 5] = [
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

pub(crate) struct CommandContext {
    pub(crate) op_id: OpId,
    pub(crate) revision_id: RevisionId,
    pub(crate) actor: Uuid7,
    pub(crate) project: ProjectId,
    pub(crate) workspace: WorkspaceId,
}

/// Parse in the established order, generating only absent identities from the caller's source.
pub(crate) fn parse(
    options: &BTreeMap<&str, &str>,
    fresh: &mut impl FnMut() -> Result<beamcraft_model::id::Uuid7, Refusal>,
) -> Result<CommandContext, Refusal> {
    let given = |key: &str| options.get(key).copied();
    Ok(CommandContext {
        op_id: match given("--op-id") {
            Some(text) => OpId::parse(text).map_err(crate::plate_cmd::context_id_error)?,
            None => OpId::from_uuid(fresh()?),
        },
        revision_id: match given("--revision-id") {
            Some(text) => RevisionId::parse(text).map_err(crate::plate_cmd::context_id_error)?,
            None => RevisionId::from_uuid(fresh()?),
        },
        actor: match given("--actor-id") {
            Some(text) => Uuid7::parse(text).map_err(Refusal::from)?,
            None => Uuid7::parse(&fresh()?.to_string()).map_err(Refusal::from)?,
        },
        project: match given("--project-id") {
            Some(text) => ProjectId::parse(text).map_err(crate::plate_cmd::context_id_error)?,
            None => ProjectId::from_uuid(fresh()?),
        },
        workspace: match given("--workspace-id") {
            Some(text) => WorkspaceId::parse(text).map_err(crate::plate_cmd::context_id_error)?,
            None => WorkspaceId::from_uuid(fresh()?),
        },
    })
}

/// The default identity of a created object when the caller names no explicit one (issue 555
/// finding 4). With an explicit `--op-id` (`op_id`) the identity is derived from it: the op's own 48-bit
/// timestamp and a tail from SHA-256 over a domain tag, the op id and `slot` (which created
/// object of the request: `new_id/0`, `new_id/1`, `section_id/0`, ...), stamped as a UUIDv7.
/// So a verbatim retry of an accepted `create-*` — the exit 4 or 124 case, where the caller
/// never saw the generated id — names the same object and replays instead of colliding with
/// `OP_ID_REUSE`. Without `--op-id` (or with an unparsable one, which the context parse refuses
/// anyway) there is nothing to retry against and the identity is fresh.
pub(crate) fn default_id(
    op_id: Option<&str>,
    slot: &str,
    fresh: &mut dyn FnMut() -> Result<beamcraft_model::id::Uuid7, Refusal>,
) -> Result<beamcraft_model::id::Uuid7, Refusal> {
    let Some(op) = op_id.and_then(|text| OpId::parse(text).ok()) else {
        return fresh();
    };
    derived_id(&op, slot).map_or_else(fresh, Ok)
}

/// The UUIDv7 [`default_id`] derives from an op id and a slot; `None` only if hashing refused
/// the (bounded) message, which a slot name cannot cause.
fn derived_id(op: &OpId, slot: &str) -> Option<beamcraft_model::id::Uuid7> {
    let mut message = Vec::with_capacity(64 + slot.len());
    message.extend_from_slice(b"beamcraft:default-id:v1\0");
    message.extend_from_slice(op.as_bytes());
    message.extend_from_slice(slot.as_bytes());
    let digest = beamcraft_model::digest::Sha256::digest(&message).ok()?;
    let mut bytes = [0u8; 16];
    for (out, (index, byte)) in bytes.iter_mut().zip(digest.as_bytes().iter().enumerate()) {
        *out = if index < 6 {
            op.as_bytes().get(index).copied().unwrap_or(0)
        } else {
            *byte
        };
    }
    if let Some(version) = bytes.get_mut(6) {
        *version = 0x70 | (*version & 0x0f);
    }
    if let Some(variant) = bytes.get_mut(8) {
        *variant = 0x80 | (*variant & 0x3f);
    }
    beamcraft_model::id::Uuid7::from_bytes(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_identity_is_derived_from_an_explicit_op_id() {
        let op = "019a7bf0-0000-7000-8000-000000000001";
        let mut fresh = || -> Result<beamcraft_model::id::Uuid7, Refusal> {
            Err(Refusal {
                rule_id: None,
                code: beamcraft_model::error::ErrorCode::BackendUnavailable,
                message: "no fresh identity in this test",
            })
        };
        let first = default_id(Some(op), "new_id/0", &mut fresh).unwrap();
        // Deterministic per op and slot, distinct between slots and from the op itself.
        assert_eq!(first, default_id(Some(op), "new_id/0", &mut fresh).unwrap());
        assert_ne!(first, default_id(Some(op), "new_id/1", &mut fresh).unwrap());
        assert_ne!(first.to_string(), op);
        // A valid UUIDv7 that keeps the op's timestamp.
        assert!(beamcraft_model::id::Uuid7::parse(&first.to_string()).is_ok());
        assert_eq!(first.to_string().get(..13), op.get(..13));
        // Without an op id the source is asked.
        assert!(default_id(None, "new_id/0", &mut fresh).is_err());
    }

    const ID: &str = "019a7bf0-0000-7000-8000-000000200001";

    #[test]
    fn explicit_context_does_not_consume_fresh_identities() {
        let options = CONTEXT_OPTIONS.into_iter().map(|key| (key, ID)).collect();
        let context = parse(&options, &mut || panic!("all identities were supplied")).unwrap();
        assert_eq!(context.op_id.to_string(), ID);
        assert_eq!(context.revision_id.to_string(), ID);
        assert_eq!(context.actor.to_string(), ID);
        assert_eq!(context.project.to_string(), ID);
        assert_eq!(context.workspace.to_string(), ID);
    }

    #[test]
    fn omitted_ids_keep_generation_order_and_share_the_callers_source() {
        let mut index = 0;
        let mut fresh = || {
            index += 1;
            Ok(
                beamcraft_model::id::Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{index:012}"))
                    .unwrap(),
            )
        };
        let context = parse(&BTreeMap::from([("--actor-id", ID)]), &mut fresh).unwrap();
        assert!(context.op_id.to_string().ends_with("000000000001"));
        assert!(context.revision_id.to_string().ends_with("000000000002"));
        assert_eq!(context.actor.to_string(), ID);
        assert!(context.project.to_string().ends_with("000000000003"));
        assert!(context.workspace.to_string().ends_with("000000000004"));
        assert!(fresh().unwrap().to_string().ends_with("000000000005"));
    }

    #[test]
    fn malformed_explicit_ids_stop_before_generation_of_later_fields() {
        for key in CONTEXT_OPTIONS {
            let mut options: BTreeMap<_, _> =
                CONTEXT_OPTIONS.into_iter().map(|k| (k, ID)).collect();
            options.insert(key, "not-an-id");
            let result = parse(&options, &mut || {
                panic!("malformed input must not generate")
            });
            let error = result.err().unwrap();
            assert_eq!(error.code, beamcraft_model::error::ErrorCode::InvalidId);
            assert_eq!(error.rule_id, None, "{key} is an envelope identity");
        }
    }

    #[test]
    fn source_failure_keeps_the_original_code_and_message() {
        let expected = Refusal {
            rule_id: None,
            code: beamcraft_model::error::ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        };
        assert_eq!(
            parse(&BTreeMap::new(), &mut || Err(expected)).err(),
            Some(expected)
        );
    }
}
