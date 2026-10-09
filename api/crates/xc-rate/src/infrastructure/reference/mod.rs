//! Reference infrastructure module.
pub mod pg_reference_batch_store;
pub use pg_reference_batch_store::{
    PgReferenceBatchStore, ReferenceBatchRecord, ReferenceLineRecord,
};
