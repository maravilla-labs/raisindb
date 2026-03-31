//\! In-memory repository management implementation.
//\!
//\! Submodules:
//\! - repo_management: Repository CRUD
//\! - branch: Branch management
//\! - revision: Revision tracking

mod branch;
mod repo_management;
mod revision;

pub use branch::InMemoryBranchRepo;
pub use repo_management::InMemoryRepositoryManagement;
pub use revision::InMemoryRevisionRepo;

#[cfg(test)]
mod tests;
