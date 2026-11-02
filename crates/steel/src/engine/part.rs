// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! One committed steel part, plate or member, behind one type (#795).
//!
//! Every typed operation of the engine exists once per kind (`copy_plate`/`copy_member`,
//! `patch_plate`/`patch_member`, `array_plate_copies`/`array_member_copies`, the two
//! evaluations and quantity records). [`PartRecord`] chooses between them here, so a command
//! adapter copies, arrays and edits a part without naming its kind. The operations are the
//! intersection of the pairs and nothing more: what exists for one kind only (contour edits,
//! corners, bends, lofted plates, welds, bolts) stays behind its own commands.

use crate::engine::command::{MaterialInfo, evaluate_plate};
use crate::engine::committed::{MemberRecord, PlateRecord};
use crate::engine::copy::{copy_member, copy_plate};
use crate::engine::instance::{copy_member_instance, copy_plate_instance};
use crate::engine::member::{ProfileInfo, evaluate_member, member_quantities};
use crate::engine::quantity::{QuantityRecord, member_record, plate_record};
use crate::engine::transform::{array_member_copies, array_plate_copies};
use crate::engine::update::{patch_member, patch_plate};
use crate::model::command::{
    ArrayElements, CopyElement, CutRecord, ElementPatch, MemberKind, PlateSide,
};
use crate::model::instance::PartKind;
use beamcraft_engine::solid::SolidMesh;
use beamcraft_model::command::{CommandError, Placement};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::id::ElementId;

/// A committed steel part with its evaluated body and quantities.
#[derive(Debug, Clone, PartialEq)]
pub enum PartRecord {
    /// A plate.
    Plate(Box<PlateRecord>),
    /// A beam or a column.
    Member(Box<MemberRecord>),
}

impl PartRecord {
    /// The part `id` among the evaluated plates and members, if it is one of them.
    pub fn find(plates: &[PlateRecord], members: &[MemberRecord], id: ElementId) -> Option<Self> {
        plates
            .iter()
            .find(|r| r.authored.id == id)
            .map(|r| Self::Plate(Box::new(r.clone())))
            .or_else(|| {
                members
                    .iter()
                    .find(|r| r.authored.id == id)
                    .map(|r| Self::Member(Box::new(r.clone())))
            })
    }

    /// The element identity.
    pub fn id(&self) -> ElementId {
        match self {
            Self::Plate(r) => r.authored.id,
            Self::Member(r) => r.authored.id,
        }
    }

    /// The element label.
    pub fn name(&self) -> &str {
        match self {
            Self::Plate(r) => &r.authored.name,
            Self::Member(r) => &r.authored.name,
        }
    }

    /// The part class number.
    pub fn class(&self) -> i32 {
        match self {
            Self::Plate(r) => r.authored.class,
            Self::Member(r) => r.authored.class,
        }
    }

    /// The pinned material definition, if any.
    pub fn material(&self) -> Option<ContentDigest> {
        match self {
            Self::Plate(r) => r.authored.material,
            Self::Member(r) => r.authored.material,
        }
    }

    /// The pinned profile definition; a plate has none.
    pub fn profile(&self) -> Option<ContentDigest> {
        match self {
            Self::Plate(_) => None,
            Self::Member(r) => Some(r.authored.profile),
        }
    }

    /// The placement of the element-local body.
    pub fn placement(&self) -> Placement {
        match self {
            Self::Plate(r) => r.authored.placement,
            Self::Member(r) => r.authored.placement,
        }
    }

    /// The authored cut records.
    pub fn cuts(&self) -> &[CutRecord] {
        match self {
            Self::Plate(r) => &r.authored.cuts,
            Self::Member(r) => &r.authored.cuts,
        }
    }

    /// The certified mesh of the body.
    pub fn solid(&self) -> &SolidMesh {
        match self {
            Self::Plate(r) => &r.body.solid,
            Self::Member(r) => &r.body.solid,
        }
    }

    /// The quantity record the derived layer stores next to the mesh.
    pub fn quantity_record(&self) -> QuantityRecord {
        match self {
            Self::Plate(r) => plate_record(&r.authored, r.body.algorithm(), &r.quantities),
            Self::Member(r) => member_record(&r.authored, &r.body.profile, &r.quantities),
        }
    }

    /// The thickness side a plate states; `None` for a member or an unstated side.
    pub fn thickness_side(&self) -> Option<PlateSide> {
        match self {
            Self::Plate(r) => r.authored.thickness_side,
            Self::Member(_) => None,
        }
    }

    /// The axis points of a member; `None` for a plate.
    pub fn axis_points_mm(&self) -> Option<&[[f64; 3]]> {
        match self {
            Self::Plate(_) => None,
            Self::Member(r) => Some(&r.authored.axis_points_mm),
        }
    }

    /// The definition kind an instance of this part names (P11).
    pub fn instance_kind(&self) -> PartKind {
        match self {
            Self::Plate(_) => PartKind::Plate,
            Self::Member(r) => match r.authored.kind {
                MemberKind::Beam => PartKind::Beam,
                MemberKind::Column => PartKind::Column,
            },
        }
    }

    /// Whether both records are the same kind with the same authored state; the evaluated body
    /// follows from it.
    pub fn same_state(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Plate(a), Self::Plate(b)) => a.authored == b.authored,
            (Self::Member(a), Self::Member(b)) => a.authored == b.authored,
            _ => false,
        }
    }

    /// The copy of this part (P05), or its instance copy (P11). The body and quantities are the
    /// source's, byte for byte.
    ///
    /// # Errors
    ///
    /// The refusal of the kind's copy.
    pub fn copied(&self, command: &CopyElement, instance: bool) -> Result<Self, CommandError> {
        Ok(match self {
            Self::Plate(source) => Self::Plate(Box::new(PlateRecord {
                authored: if instance {
                    copy_plate_instance(&source.authored, command)?
                } else {
                    copy_plate(&source.authored, command)?
                },
                body: source.body.clone(),
                quantities: source.quantities,
            })),
            Self::Member(source) => Self::Member(Box::new(MemberRecord {
                authored: if instance {
                    copy_member_instance(&source.authored, command)?
                } else {
                    copy_member(&source.authored, command)?
                },
                body: source.body.clone(),
                quantities: source.quantities,
            })),
        })
    }

    /// The array copies of this part (T03), each with the source's body and quantities.
    ///
    /// # Errors
    ///
    /// The refusal of the kind's array.
    pub fn arrayed(&self, command: &ArrayElements) -> Result<Vec<Self>, CommandError> {
        Ok(match self {
            Self::Plate(source) => array_plate_copies(&source.authored, command)?
                .into_iter()
                .map(|authored| {
                    Self::Plate(Box::new(PlateRecord {
                        authored,
                        body: source.body.clone(),
                        quantities: source.quantities,
                    }))
                })
                .collect(),
            Self::Member(source) => array_member_copies(&source.authored, command)?
                .into_iter()
                .map(|authored| {
                    Self::Member(Box::new(MemberRecord {
                        authored,
                        body: source.body.clone(),
                        quantities: source.quantities,
                    }))
                })
                .collect(),
        })
    }

    /// This part after `patch` (P04), with its body re-evaluated when `shape` says the patch
    /// changes the local shape and its quantities taken for the patched material.
    ///
    /// A member keeps its standing profile unless the patch names a new one.
    ///
    /// # Errors
    ///
    /// The refusal of the kind's patch (a field of the other kind is `UNSUPPORTED_PROPERTY`), of
    /// a definition lookup, or of the evaluation.
    pub fn patched(
        &self,
        patch: &ElementPatch,
        shape: bool,
        material: impl Fn(&ContentDigest) -> Result<MaterialInfo, CommandError>,
        profile: impl Fn(&ContentDigest) -> Result<ProfileInfo, CommandError>,
    ) -> Result<Self, CommandError> {
        Ok(match self {
            Self::Plate(head) => {
                let authored = patch_plate(&head.authored, patch)?;
                let info = authored.material.as_ref().map(&material).transpose()?;
                let body = if shape {
                    evaluate_plate(&authored)?
                } else {
                    head.body.clone()
                };
                let quantities = body.quantities_for(info)?;
                Self::Plate(Box::new(PlateRecord {
                    authored,
                    body,
                    quantities,
                }))
            }
            Self::Member(head) => {
                let authored = patch_member(&head.authored, patch)?;
                let info = authored.material.as_ref().map(&material).transpose()?;
                // A path patch (P12) is the supported member shape edit: the body is
                // re-evaluated from the resolved profile. The other supported member edits
                // change metadata, placement or material, never local shape.
                let body = if shape {
                    // A profile patch rebuilds the body from the new section; any other shape
                    // patch keeps the standing one.
                    let resolved = match patch.profile {
                        Some(digest) => profile(&digest)?,
                        None => head.body.profile.clone(),
                    };
                    evaluate_member(&authored, resolved)?
                } else {
                    head.body.clone()
                };
                let quantities = member_quantities(&body, info)?;
                Self::Member(Box::new(MemberRecord {
                    authored,
                    body,
                    quantities,
                }))
            }
        })
    }

    /// Another instance of the same shared definition (P11) after the shape and material part
    /// of a patch: its own authored state patched, the body and quantities of `self`, the
    /// already patched instance. `None` when `head` is not the same kind.
    ///
    /// # Errors
    ///
    /// The refusal of the kind's patch.
    pub fn shared_with(
        &self,
        head: &Self,
        patch: &ElementPatch,
    ) -> Result<Option<Self>, CommandError> {
        Ok(match (self, head) {
            (Self::Plate(next), Self::Plate(head)) => Some(Self::Plate(Box::new(PlateRecord {
                authored: patch_plate(&head.authored, patch)?,
                body: next.body.clone(),
                quantities: next.quantities,
            }))),
            (Self::Member(next), Self::Member(head)) => {
                Some(Self::Member(Box::new(MemberRecord {
                    authored: patch_member(&head.authored, patch)?,
                    body: next.body.clone(),
                    quantities: next.quantities,
                })))
            }
            _ => None,
        })
    }

    /// The integrity message when another instance of this part's definition was not evaluated.
    pub const fn unevaluated_instance(&self) -> &'static str {
        match self {
            Self::Plate(_) => "shared plate is not evaluated",
            Self::Member(_) => "shared member is not evaluated",
        }
    }
}
