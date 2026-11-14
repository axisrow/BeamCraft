// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Shared JSON result envelope for commands using the standard store-command contract.

use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use session::Refusal;

type Run<T> = Result<T, Refusal>;

/// What a print site writes instead of a reply that does not fit the shared JSON encoder
/// budgets (1 MiB of text, 65 536 tokens of scratch): the `code` any other refusal carries
/// plus one message naming the budget, so the client learns why the reply is empty. One
/// constant keeps the one-process, session and framed replies identical.
pub(crate) const REPLY_OVER_BUDGET_LINE: &str = "{\"outcome\":\"rejected\",\"code\":\"LIMIT_EXCEEDED\",\"message\":\"the reply exceeds the output budget\"}";

/// The shared result shape of the store commands: the JSON object and the exit code (0 success,
/// 2 rejection, 3 backend unavailable, 4 outcome unknown).
pub(crate) fn report(result: Run<Json>) -> (Json, u8) {
    report_with(result, Vec::new())
}

/// [`report`] with the `diagnostics` array of a rejection filled in (the candidates of an
/// ambiguous name, or the F10 domain finding of a ruled rejection).
pub(crate) fn report_with(result: Run<Json>, mut diagnostics: Vec<Json>) -> (Json, u8) {
    match result {
        Ok(json) => (json, 0),
        Err(e) => {
            // A rejection carrying an F10 rule id reaches the client as a domain finding.
            diagnostics.extend(crate::plate_cmd::domain_diagnostics(&e));
            (
                Json::object([
                    (
                        "outcome",
                        Json::text(if e.code == ErrorCode::OutcomeUnknown {
                            "outcome_unknown"
                        } else {
                            "rejected"
                        }),
                    ),
                    ("code", Json::text(e.code.as_str())),
                    ("message", Json::text(e.message)),
                    ("observed_context", Json::Null),
                    ("diagnostics", Json::Array(diagnostics)),
                ]),
                match e.code {
                    ErrorCode::OutcomeUnknown => 4,
                    ErrorCode::BackendUnavailable => 3,
                    _ => 2,
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_keeps_its_outcome_and_process_status() {
        for code in ErrorCode::ALL {
            let (json, exit) = report(Err(Refusal {
                rule_id: None,
                code,
                message: "command-specific message",
            }));
            let (outcome, expected_exit) = match code {
                ErrorCode::OutcomeUnknown => ("outcome_unknown", 4),
                ErrorCode::BackendUnavailable => ("rejected", 3),
                _ => ("rejected", 2),
            };
            assert_eq!(exit, expected_exit);
            let expected = Json::object([
                ("outcome", Json::text(outcome)),
                ("code", Json::text(code.as_str())),
                ("message", Json::text("command-specific message")),
                ("observed_context", Json::Null),
                ("diagnostics", Json::Array(Vec::new())),
            ]);
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn supplied_diagnostics_survive_reporting_and_encoding() {
        let diagnostics = vec![Json::object([("candidate", Json::text("original value"))])];
        let (json, exit) = report_with(
            Err(Refusal {
                rule_id: None,
                code: ErrorCode::InvalidArgument,
                message: "ambiguous name",
            }),
            diagnostics.clone(),
        );
        assert_eq!(exit, 2);
        let encoded = json.encode(&beamcraft_json::Limits::DEFAULT).unwrap();
        let decoded = beamcraft_json::decode(&encoded, &beamcraft_json::Limits::DEFAULT).unwrap();
        assert_eq!(
            decoded.get("diagnostics").unwrap(),
            &Json::Array(diagnostics)
        );
    }

    #[test]
    fn success_is_returned_without_an_envelope() {
        let value = Json::object([("outcome", Json::text("command-specific success"))]);
        assert_eq!(report(Ok(value.clone())), (value, 0));
    }

    #[test]
    fn a_reply_over_the_encoder_budget_refuses_with_a_message() {
        // One byte over the 1 MiB text budget already fails to encode; the fallback line that
        // replaces such a reply decodes and names the budget instead of leaving a bare code.
        let oversized = Json::text("x".repeat(beamcraft_json::Limits::DEFAULT.bytes + 1));
        assert!(oversized.encode(&beamcraft_json::Limits::DEFAULT).is_err());
        let decoded = beamcraft_json::decode(
            REPLY_OVER_BUDGET_LINE.as_bytes(),
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        assert_eq!(decoded.get("code").unwrap(), &Json::text("LIMIT_EXCEEDED"));
        assert_eq!(
            decoded.get("message").unwrap(),
            &Json::text("the reply exceeds the output budget")
        );
    }
}
