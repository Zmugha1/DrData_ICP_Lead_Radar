pub mod assignments;
pub mod attributes;
pub mod owner_type;
pub mod boundaries;
pub mod coverage;
pub mod entity;
pub mod error;
pub mod load;
pub mod manifest;
pub mod search_query_templates;
pub mod series;
pub mod session;
pub mod sources;
pub mod tiers;
pub mod triggers;
pub mod validate;
pub mod vocabulary;
pub mod work_scope;
pub mod zones;

pub use assignments::AssignmentsFile;
pub use attributes::{Attribute, Attributes, Band, Direction, Favourability, ValueType};
pub use owner_type::{
    CompiledOwnerType, OwnerTypeClass, OwnerTypeFile, OwnerTypePhrase, PhraseAnchor,
};
pub use boundaries::{AbsenceRule, AbsenceRuleKind, Boundaries};
pub use coverage::{Coverage, CoverageEntry, CoverageLevel, PublicationLag};
pub use entity::{Attachment, Entity, FieldMap, JoinMethod, Layer, LayerRole, LookupSource};
pub use error::ConfigError;
pub use load::{check_config_dir, config_fingerprint, load, ConfigDirError, REQUIRED_CONFIG_FILES};
pub use manifest::{
    default_pages, nav_pages, page_is_enabled, EntityUniverse, Manifest, NavPage, PageEntry,
    PAGE_NOT_ENABLED, REQUIRED_PAGE_KEYS,
};
pub use search_query_templates::{SearchQueryCategory, SearchQueryTemplates};
pub use series::{SeriesEntry, SeriesFile};
pub use session::SessionState;
pub use sources::{SourceEntry, SourceFamily, SourceKind, SourceStatus, SourcesFile};
pub use tiers::{Tier, TiersFile};
pub use triggers::{Trigger, Triggers};
pub use validate::{validate, Finding, Severity};
pub use vocabulary::{VerticalVocabulary, Vocabulary};
pub use work_scope::{Capacity, Job, WorkScope};
pub use zones::{Zone, ZoneAssignment, Zones};

#[derive(Debug, Clone, PartialEq)]
pub struct RadarConfig {
    pub manifest: Manifest,
    pub entity: Entity,
    pub attributes: Attributes,
    pub triggers: Triggers,
    pub work_scope: WorkScope,
    pub vocabulary: Vocabulary,
    pub zones: Zones,
    pub boundaries: Boundaries,
    pub coverage: Coverage,
    pub sources: SourcesFile,
    pub series: SeriesFile,
    pub assignments: AssignmentsFile,
    pub owner_type: OwnerTypeFile,
    pub tiers: TiersFile,
    pub search_query_templates: Option<SearchQueryTemplates>,
}

#[cfg(test)]
mod tests;
