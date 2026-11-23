// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A07.2 and A07.3 headless commands: `list-library`, `show-library` and `add-library`. They read
//! the files named on the command line (one `--library`, an `--environment` with its libraries,
//! or a project's own layered chain through `--project`; never a hidden global), never the
//! model, and print one JSON object with the error shape and exit codes of the plate commands.
//! Only `add-library` writes, and only the one library file it names.
//!
//! A project's chain carries three namespaces, profiles, fasteners and materials, over the same
//! layers. Without `--namespace` the two listing commands answer with the profiles, as before;
//! `--namespace` names one of them or, for `list-library`, all three paged as one position
//! space in that order. The resolvers are never merged into one lookup: the same text may be a
//! grade and a designation, so a merged lookup would report shadowing where there is none.

use crate::plate_cmd::{
    Run, definition_json, environment_lost_json, fit_page, invalid, page_of, profile_record,
    shadow_json,
};
use crate::report::report;
use beamcraft_commands::library::{
    EnvironmentCopyLost, FirmFileRefused, LAYER_LIBRARY, LibraryEntry, LibraryInfo, Lookup,
    Namespace, NewLibrary, RejectedEntry, Shadowed, Source, add_entry, load_fastener, open_lookup,
    project_library_chain,
};
use beamcraft_json::Json;
use session::environment::EnvironmentInfo;
use std::collections::BTreeMap;
use std::ffi::OsString;

/// Runs one library command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    report(run(args))
}

fn optional(value: &Option<String>) -> Json {
    value.as_ref().map_or(Json::Null, Json::text)
}

fn info_json(info: &LibraryInfo) -> Json {
    Json::object([
        ("id", Json::text(info.id.clone())),
        ("title", optional(&info.title)),
        ("version", optional(&info.version)),
        ("source", optional(&info.source)),
        ("licence", optional(&info.licence)),
        ("digest", Json::text(info.digest.to_string())),
        ("bytes", Json::uint(info.bytes)),
    ])
}

fn environment_json(info: &EnvironmentInfo) -> Json {
    let mut object = BTreeMap::from([
        ("id".to_owned(), Json::text(info.id.clone())),
        ("title".to_owned(), optional(&info.title)),
        ("digest".to_owned(), Json::text(info.digest.to_string())),
        ("bytes".to_owned(), Json::uint(info.bytes)),
        (
            "libraries".to_owned(),
            Json::Array(info.libraries.iter().map(Json::text).collect()),
        ),
        (
            "fastener_libraries".to_owned(),
            Json::Array(info.fastener_libraries.iter().map(Json::text).collect()),
        ),
        (
            "material_libraries".to_owned(),
            Json::Array(info.material_libraries.iter().map(Json::text).collect()),
        ),
    ]);
    if let Some(version) = &info.version {
        object.insert("version".into(), Json::text(version.clone()));
    }
    Json::Object(object)
}

fn source_json(source: &Source) -> Json {
    Json::object([
        ("path", Json::text(source.path.clone())),
        ("layer", Json::text(source.layer)),
        ("library", info_json(&source.library.info)),
        (
            "entry_count",
            Json::uint(source.library.entries.len() as u64),
        ),
        (
            "rejected_count",
            Json::uint(source.library.rejected.len() as u64),
        ),
    ])
}

fn entry_json(entry: &LibraryEntry, library_id: &str) -> Run<Json> {
    let digest = entry.record.resource()?.digest;
    let Json::Object(mut object) = definition_json(&digest, &entry.record)? else {
        return Err(invalid("entry result is not an object"));
    };
    // The digest is what `define-profile --library` would pin; the library prim is not a
    // definition path.
    object.remove("prim_path");
    object.insert("library_prim".into(), Json::text(entry.prim_path.clone()));
    object.insert("library_id".into(), Json::text(library_id));
    object.insert("designation".into(), Json::text(entry.designation.clone()));
    Ok(Json::Object(object))
}

fn rejected_json<'a>(rejected: impl Iterator<Item = (&'a str, &'a RejectedEntry)>) -> Json {
    Json::Array(
        rejected
            .map(|(library_id, r)| {
                Json::object([
                    ("library_id", Json::text(library_id)),
                    ("prim_path", Json::text(r.prim_path.clone())),
                    (
                        "designation",
                        r.designation.as_ref().map_or(Json::Null, Json::text),
                    ),
                    ("code", Json::text(r.code.as_str())),
                    ("message", Json::text(r.message)),
                ])
            })
            .collect(),
    )
}

fn firm_refused_json(refused: &[FirmFileRefused]) -> Json {
    Json::Array(
        refused
            .iter()
            .map(|r| {
                Json::object([
                    ("path", Json::text(r.path.clone())),
                    ("code", Json::text(r.code.as_str())),
                    ("message", Json::text(r.message)),
                ])
            })
            .collect(),
    )
}

const PROFILE_OPTIONS: &[&str] = &[
    "--width",
    "--height",
    "--family",
    "--h",
    "--b",
    "--tw",
    "--tf",
    "--t",
    "--r",
    "--r1",
    "--c",
    "--d",
    "--s",
    "--k",
    "--outer",
    "--cavity",
    "--designation",
    "--standard",
    "--area-mm2",
    "--mass-per-meter-kg",
    "--surface-per-meter-m2",
];

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    let lookup_options = [
        "--library",
        "--environment",
        "--user-library",
        "--fastener-library",
        "--project",
    ];
    let allowed: Vec<&str> = match *command {
        "list-library" => [
            &lookup_options[..],
            &[
                "--family",
                "--filter",
                "--offset",
                "--limit",
                "--shadow-offset",
                "--namespace",
            ],
        ]
        .concat(),
        "show-library" => [&lookup_options[..], &["--designation", "--namespace"]].concat(),
        "add-library" => [
            &["--library", "--library-id", "--title"][..],
            PROFILE_OPTIONS,
        ]
        .concat(),
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 48 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    if *command == "add-library" {
        return add(&options);
    }
    // The namespaces are those of a project chain: a named file is one namespace already. One
    // namespace answers a lookup and is the default of a listing; `all` lists every namespace
    // of the chain, profiles, fasteners, materials, as one position space.
    let tagged = options.contains_key("--namespace");
    let (first, rest): (Namespace, Vec<Namespace>) = match options.get("--namespace").copied() {
        None => (Namespace::Profile, Vec::new()),
        Some(_) if !options.contains_key("--project") => {
            return Err(invalid("--namespace needs --project"));
        }
        Some("all") if *command == "show-library" => {
            return Err(invalid(
                "--namespace all lists every namespace; show-library takes one of profile, fastener or material",
            ));
        }
        Some("all") => (
            Namespace::Profile,
            vec![Namespace::Fastener, Namespace::Material],
        ),
        Some(token) => (
            Namespace::parse(token).ok_or_else(|| {
                invalid("--namespace is one of profile, fastener, material or all")
            })?,
            Vec::new(),
        ),
    };
    if let Some(store) = options.get("--project") {
        // The project's own layered chain: its environment copy, the firm libraries of the
        // application data, and the built-in neutral environment. No store is opened; only the
        // copy's files are read. A namespace no layer of the chain declares answers nothing.
        if [
            "--library",
            "--environment",
            "--fastener-library",
            "--user-library",
        ]
        .iter()
        .any(|used| options.contains_key(*used))
        {
            return Err(invalid(
                "--project is an alternative to --library, --environment, --fastener-library and --user-library",
            ));
        }
        let chain = project_library_chain(std::path::Path::new(store), None)?;
        let of = |namespace: Namespace| {
            (
                tagged.then_some(namespace),
                chain.lookup_of(namespace).cloned().unwrap_or_default(),
            )
        };
        return finish(
            command,
            &options,
            of(first),
            rest.into_iter().map(of).collect(),
            chain.environment,
            chain.firm_refused,
            chain.environment_lost,
        );
    }
    // A14.1: a fastener library is a single file, no environment yet (O-B2); the same listing
    // and lookup machinery answers it.
    let (lookup, environment, firm_refused) = if let Some(path) = options.get("--fastener-library")
    {
        if ["--library", "--environment", "--user-library", "--project"]
            .iter()
            .any(|used| options.contains_key(*used))
        {
            return Err(invalid(
                "--fastener-library is an alternative to --library, --environment, --user-library and --project",
            ));
        }
        let library = load_fastener(path)?;
        (
            Lookup::new(vec![Source {
                path: (*path).to_owned(),
                library,
                layer: LAYER_LIBRARY,
            }]),
            None,
            Vec::new(),
        )
    } else {
        let opened = open_lookup(
            options.get("--library").copied(),
            options.get("--environment").copied(),
            options.get("--user-library").copied(),
            None,
        )?;
        (opened.lookup, opened.environment, opened.firm_refused)
    };
    finish(
        command,
        &options,
        (None, lookup),
        Vec::new(),
        environment,
        firm_refused,
        None,
    )
}

/// The reply of `list-library` and `show-library` over the resolvers that were built: the entry
/// view, the shadowed designations with their layers, the firm files that did not read, and the
/// project's lost environment copy when the chain reports one.
///
/// An exact lookup takes `first`, the one namespace that was asked for; a listing pages `first`,
/// then `rest`, as one position space. Each resolver carries the namespace its entries are
/// listed under, `None` when `--namespace` was not given.
fn finish(
    command: &&str,
    options: &BTreeMap<&str, &str>,
    first: (Option<Namespace>, Lookup),
    rest: Vec<(Option<Namespace>, Lookup)>,
    environment: Option<EnvironmentInfo>,
    firm_refused: Vec<FirmFileRefused>,
    environment_lost: Option<EnvironmentCopyLost>,
) -> Run<Json> {
    if *command == "show-library" {
        let (namespace, lookup) = &first;
        return show(
            options,
            *namespace,
            lookup,
            &firm_refused,
            environment_lost.as_ref(),
        );
    }
    let resolvers: Vec<(Option<Namespace>, Lookup)> = std::iter::once(first).chain(rest).collect();
    list(
        options,
        &resolvers,
        environment.as_ref(),
        &firm_refused,
        environment_lost.as_ref(),
    )
}

/// `show-library`: the entry that answers the exact designation, with its library. Under
/// `--namespace` the reply and its entry also name the namespace and the layer that answer.
fn show(
    options: &BTreeMap<&str, &str>,
    namespace: Option<Namespace>,
    lookup: &Lookup,
    firm_refused: &[FirmFileRefused],
    environment_lost: Option<&EnvironmentCopyLost>,
) -> Run<Json> {
    let designation = options
        .get("--designation")
        .ok_or_else(|| invalid("--designation is required"))?;
    let (source, entry) = lookup.find(designation)?;
    let mut entry = entry_json(entry, &source.library.info.id)?;
    let mut object = BTreeMap::from([
        ("outcome".to_owned(), Json::text("library_entry")),
        ("library".to_owned(), info_json(&source.library.info)),
        ("library_path".to_owned(), Json::text(source.path.clone())),
        (
            "shadowed".to_owned(),
            shadow_json(lookup.shadowed_for(designation)),
        ),
    ]);
    if let Some(namespace) = namespace {
        let (layer, namespace) = (Json::text(source.layer), Json::text(namespace.token()));
        object.insert("layer".to_owned(), layer.clone());
        object.insert("namespace".to_owned(), namespace.clone());
        if let Json::Object(fields) = &mut entry {
            fields.insert("layer".to_owned(), layer);
            fields.insert("namespace".to_owned(), namespace);
        }
    }
    object.insert("entry".to_owned(), entry);
    if !firm_refused.is_empty() {
        object.insert("firm_refused".to_owned(), firm_refused_json(firm_refused));
    }
    if let Some(lost) = environment_lost {
        object.insert("environment_lost".to_owned(), environment_lost_json(lost));
    }
    Ok(Json::Object(object))
}

/// `list-library`: the entries that answer in every resolver, in order, then the refused ones,
/// as one position space. The shadow records, the libraries and the refused entries of every
/// resolver follow the same order; the resolvers are never merged into one lookup.
fn list(
    options: &BTreeMap<&str, &str>,
    resolvers: &[(Option<Namespace>, Lookup)],
    environment: Option<&EnvironmentInfo>,
    firm_refused: &[FirmFileRefused],
    environment_lost: Option<&EnvironmentCopyLost>,
) -> Run<Json> {
    let family = options.get("--family");
    let filter = options.get("--filter").copied();
    let (offset, limit) = page_of(options)?;
    let sources = || resolvers.iter().flat_map(|(_, lookup)| lookup.sources());
    let matching: Vec<(Option<Namespace>, &Source, &LibraryEntry)> = resolvers
        .iter()
        .flat_map(|(namespace, lookup)| {
            lookup
                .matching(filter)
                .into_iter()
                .filter(|(_, e)| family.is_none_or(|f| e.record.kind() == *f))
                .map(move |(source, entry)| (*namespace, source, entry))
        })
        .collect();
    let refused: Vec<(&str, &RejectedEntry)> = sources()
        .flat_map(|s| {
            s.library
                .rejected
                .iter()
                .map(|r| (s.library.info.id.as_str(), r))
        })
        .collect();
    // The shadow records are paged on their own (`--shadow-offset`, `--limit` of them): a few
    // overlapping libraries of thousands of entries make tens of thousands of records, which
    // must not decide whether the entries fit the output budget.
    let shadows: Vec<Shadowed> = resolvers
        .iter()
        .flat_map(|(_, lookup)| lookup.shadowed())
        .collect();
    let shadow_offset = match options.get("--shadow-offset") {
        Some(text) => text
            .parse::<usize>()
            .map_err(|_| invalid("--shadow-offset is not a number"))?,
        None => 0,
    };
    let shadow_end = shadow_offset.saturating_add(limit).min(shadows.len());
    let shadow_page: Vec<Shadowed> = shadows
        .get(shadow_offset..shadow_end)
        .unwrap_or_default()
        .to_vec();
    let shadow_next = if shadow_end < shadows.len() {
        Json::uint(shadow_end as u64)
    } else {
        Json::Null
    };
    let shadow_total = shadows.len() as u64;
    // One position space: the matching entries of every resolver, then the refused ones.
    let (entry_total, rejected_total) = (matching.len(), refused.len());
    let total = entry_total.saturating_add(rejected_total);
    fit_page(total.saturating_sub(offset), limit, |count| {
        let entries_here = entry_total.saturating_sub(offset).min(count);
        let entries = matching
            .iter()
            .skip(offset)
            .take(entries_here)
            .map(|(namespace, source, entry)| {
                let mut object = entry_json(entry, &source.library.info.id)?;
                if let Json::Object(fields) = &mut object {
                    fields.insert("layer".into(), Json::text(source.layer));
                    // Under --namespace every entry names the namespace it was listed from.
                    if let Some(namespace) = namespace {
                        fields.insert("namespace".into(), Json::text(namespace.token()));
                    }
                }
                Ok(object)
            })
            .collect::<Run<Vec<_>>>()?;
        let rejected = rejected_json(
            refused
                .iter()
                .skip(offset.saturating_sub(entry_total))
                .take(count - entries_here)
                .copied(),
        );
        let next = offset.saturating_add(count);
        let mut object = BTreeMap::from([
            ("outcome".to_owned(), Json::text("library")),
            ("entry_count".to_owned(), Json::uint(entry_total as u64)),
            (
                "rejected_count".to_owned(),
                Json::uint(rejected_total as u64),
            ),
            ("offset".to_owned(), Json::uint(offset as u64)),
            ("page_size".to_owned(), Json::uint(count as u64)),
            (
                "next_offset".to_owned(),
                if next < total {
                    Json::uint(next as u64)
                } else {
                    Json::Null
                },
            ),
            ("entries".to_owned(), Json::Array(entries)),
            ("rejected".to_owned(), rejected),
            (
                "libraries".to_owned(),
                Json::Array(sources().map(source_json).collect()),
            ),
            ("shadowed".to_owned(), shadow_json(shadow_page.clone())),
            ("shadowed_count".to_owned(), Json::uint(shadow_total)),
            ("shadowed_next_offset".to_owned(), shadow_next.clone()),
        ]);
        // One resolver of one library and no environment names it, whatever namespace was
        // asked for: under --namespace the reply is the untagged one plus the namespace keys.
        if let [(_, lookup)] = resolvers
            && let [only] = lookup.sources()
            && environment.is_none()
        {
            object.insert("library".to_owned(), info_json(&only.library.info));
        }
        if let Some(environment) = environment {
            object.insert("environment".to_owned(), environment_json(environment));
        }
        if let Some(lost) = environment_lost {
            object.insert("environment_lost".to_owned(), environment_lost_json(lost));
        }
        if !firm_refused.is_empty() {
            object.insert("firm_refused".to_owned(), firm_refused_json(firm_refused));
        }
        Ok(Json::Object(object))
    })
}

/// `add-library`: one profile entry into the library file named by `--library`, creating the file
/// when `--library-id` is given.
fn add(options: &BTreeMap<&str, &str>) -> Run<Json> {
    let path = options
        .get("--library")
        .ok_or_else(|| invalid("--library is required"))?;
    let designation = options
        .get("--designation")
        .ok_or_else(|| invalid("--designation is required"))?;
    let new = match (options.get("--library-id"), options.get("--title")) {
        (Some(id), title) => Some(NewLibrary {
            id: (*id).to_owned(),
            title: title.map(|t| (*t).to_owned()),
        }),
        (None, Some(_)) => return Err(invalid("--title needs --library-id")),
        (None, None) => None,
    };
    let record = profile_record(options)?;
    let added = add_entry(path, record, designation, new)?;
    Ok(Json::object([
        ("outcome", Json::text("library_entry_added")),
        ("created", Json::Bool(added.created)),
        ("library", info_json(&added.info)),
        ("entry_count", Json::uint(added.entry_count as u64)),
        ("entry", entry_json(&added.entry, &added.info.id)?),
    ]))
}
