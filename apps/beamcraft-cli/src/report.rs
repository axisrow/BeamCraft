// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Shared JSON result envelope for commands using the standard store-command contract.

use beamcraft_cli::plate::PlateError;
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;

type Run<T> = Result<T, PlateError>;

/// The shared result shape of the store commands: the JSON object and the exit code (0 success,
/// 2 rejection, 3 backend unavailable, 4 outcome unknown).
pub(crate) fn report(result: Run<Json>) -> (Json, u8) {
    report_with(result, Vec::new())
}

/// [`report`] with the `diagnostics` array of a rejection filled in (the candidates of an
/// ambiguous name).
pub(crate) fn report_with(result: Run<Json>, diagnostics: Vec<Json>) -> (Json, u8) {
    match result {
        Ok(json) => (json, 0),
        Err(e) => (
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
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_keeps_its_outcome_and_process_status() {
        for code in ErrorCode::ALL {
            let (json, exit) = report(Err(PlateError {
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
            Err(PlateError {
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
}
