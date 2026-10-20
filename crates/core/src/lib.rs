//! AgentTidy core domain models (`Start.md` §9).
//!
//! This crate holds the provider-neutral domain types shared by the whole
//! system: agent installations, capabilities, sessions, resources and the
//! space accounting model (§8). Providers (`crates/provider-api`, `providers/`)
//! describe facts in these terms; cleanup policy and planning live here too
//! (later phases). Platform-specific code is forbidden in this crate
//! (architecture red line #11, `Start.md` §22).
//!
//! Design rule (§19 Phase 1): these types are the *read-only analysis*
//! vocabulary only — no cleanup/execution types yet (those land in Phase 6
//! after the executor protocol is designed).

pub mod capability;
pub mod cleanup;
pub mod installation;
pub mod provider;
pub mod registry;
pub mod resource;
pub mod session;
pub mod size;
pub mod snapshot;

pub use capability::{AgentCapabilities, CapabilityStatus, CapabilityTopic};
pub use cleanup::{
    CleanupAction, CleanupDecision, CleanupEvent, CleanupEvidence, CleanupItem, CleanupItemOutcome,
    CleanupLocator, CleanupOperation, CleanupOutcome, CleanupPlan, CleanupPrecondition,
    CleanupPreconditionKind, CleanupRevalidationOutcome, CleanupUnit, CleanupUnitKind,
    FileIdentity, ResourceFingerprint, RiskLevel, RiskSummary,
};
pub use installation::{AgentInstallation, InstallationStatus, Platform};
pub use provider::ProviderId;
pub use registry::{DuplicateProvider, ProviderRegistry};
pub use resource::{
    ManagedBy, Ownership, Resource, ResourceId, ResourceKind, ResourceLocator, ResourceRef,
};
pub use session::{ProjectRef, Session, SessionId, SessionLifecycle};
pub use size::{SizeConfidence, SizeInfo};
pub use snapshot::{AgentSnapshot, ScanOptions, ScanProblem};
