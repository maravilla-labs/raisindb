// SPDX-License-Identifier: BSL-1.1

//! IndexingEngine trait implementation for TantivyIndexingEngine.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::fulltext::{
    FullTextIndexJob, FullTextSearchQuery, FullTextSearchResult, IndexingEngine, NodeIndexPlan,
};

use super::document::create_document;
use super::properties::flatten_properties;
use super::search::execute_search;
use super::types::TantivyIndexingEngine;
use super::utils::snapshot_index_dir;

impl IndexingEngine for TantivyIndexingEngine {
    fn do_index_node_with_plan(
        &self,
        job: &FullTextIndexJob,
        node: &Node,
        plan: &NodeIndexPlan,
    ) -> Result<()> {
        let cached = self.get_or_create_index(&job.tenant_id, &job.repo_id, &job.branch)?;
        let index = &cached.index;
        let default_lang = &job.default_language;

        let fields = &cached.fields;
        let flattened = flatten_properties(plan, &node.properties);

        let default_doc = create_document(
            job,
            node,
            default_lang,
            &node.name,
            &flattened.content,
            &flattened.shape_types,
            fields,
        );
        let mut documents = vec![default_doc];

        if let Some(translations) = &node.translations {
            for lang_code in &job.supported_languages {
                if lang_code == default_lang {
                    continue;
                }

                let translated_name = translations
                    .get(&format!("name_{}", lang_code))
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or(&node.name);

                let doc = create_document(
                    job,
                    node,
                    lang_code,
                    translated_name,
                    &flattened.content,
                    &flattened.shape_types,
                    fields,
                );
                documents.push(doc);
            }
        }

        let index_key = Self::index_key(&job.tenant_id, &job.repo_id, &job.branch);
        self.with_writer(&index_key, index, |writer| {
            // Delete first, add after: tantivy applies a delete only to documents
            // added BEFORE it, so the replacements below survive their own delete.
            let node_id_term = tantivy::Term::from_field_text(fields.node_id, &node.id);
            writer.delete_term(node_id_term);

            for doc in documents {
                writer
                    .add_document(doc)
                    .map_err(|e| Error::storage(format!("Failed to add document: {}", e)))?;
            }
            Ok(())
        })
    }

    fn do_delete_node(&self, job: &FullTextIndexJob) -> Result<()> {
        let cached = self.get_or_create_index(&job.tenant_id, &job.repo_id, &job.branch)?;
        let index = &cached.index;

        let fields = &cached.fields;
        let index_key = Self::index_key(&job.tenant_id, &job.repo_id, &job.branch);

        self.with_writer(&index_key, index, |writer| {
            if let Some(node_id) = &job.node_id {
                let node_id_term = tantivy::Term::from_field_text(fields.node_id, node_id);
                writer.delete_term(node_id_term);
            }
            Ok(())
        })
    }

    fn do_branch_created(&self, job: &FullTextIndexJob) -> Result<()> {
        let source_branch = job.source_branch.as_ref().ok_or_else(|| {
            Error::Validation("source_branch is required for branch_created operation".to_string())
        })?;

        let source_path = self
            .base_path
            .join(&job.tenant_id)
            .join(&job.repo_id)
            .join(source_branch);

        let target_path = self
            .base_path
            .join(&job.tenant_id)
            .join(&job.repo_id)
            .join(&job.branch);

        if !source_path.exists() {
            return Err(Error::NotFound(format!(
                "Source branch index not found: {}",
                source_branch
            )));
        }

        // Quiesce the source before reading its directory. Every write path
        // commits before it releases the writer, so taking the writer is enough
        // to guarantee we are not snapshotting an index caught between
        // `add_document` and `commit`. The commit itself is a no-op in the
        // common case and cheap when it is not.
        //
        // This does NOT stop merges — those land on the segment updater's own
        // threads — which is why `snapshot_index_dir` validates the result
        // rather than trusting the copy.
        {
            let source_cached =
                self.get_or_create_index(&job.tenant_id, &job.repo_id, source_branch)?;
            let source_key = Self::index_key(&job.tenant_id, &job.repo_id, source_branch);
            self.with_writer(&source_key, &source_cached.index, |_writer| Ok(()))?;
        }

        snapshot_index_dir(&source_path, &target_path)?;

        Ok(())
    }

    fn search(&self, query: &FullTextSearchQuery) -> Result<Vec<FullTextSearchResult>> {
        execute_search(self, query)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tantivy_engine::TantivyIndexingEngine;
    use raisin_storage::fulltext::JobKind;
    use std::collections::HashMap;

    fn node(i: usize) -> Node {
        Node {
            id: format!("node-{i}"),
            name: format!("Node {i}"),
            path: format!("/n/{i}"),
            node_type: "ns:Page".to_string(),
            archetype: None,
            properties: HashMap::new(),
            children: Vec::new(),
            parent: None,
            version: 1,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: None,
            owner_id: None,
            has_children: None,
            order_key: String::new(),
            relations: Default::default(),
        }
    }

    fn job(node_id: &str) -> FullTextIndexJob {
        FullTextIndexJob {
            job_id: "j".to_string(),
            kind: JobKind::AddNode,
            tenant_id: "t".to_string(),
            repo_id: "r".to_string(),
            workspace_id: "w".to_string(),
            branch: "main".to_string(),
            revision: raisin_hlc::HLC::new(1, 0),
            node_id: Some(node_id.to_string()),
            source_branch: None,
            default_language: "en".to_string(),
            supported_languages: vec!["en".to_string()],
            properties_to_index: None,
        }
    }

    fn plan() -> NodeIndexPlan {
        NodeIndexPlan {
            node_type: "ns:Page".to_string(),
            archetype: None,
            top_level_props: Some(vec![]),
            element_plans: HashMap::new(),
            legacy_index_all_strings: false,
        }
    }

    /// A branch copy must produce an index that opens and reads, taken from a
    /// source that has a live writer attached and merges in flight.
    ///
    /// The copy used to be a plain recursive `fs::copy` performed straight
    /// into the branch's own path, so a merge landing mid-copy could leave the
    /// new branch with a `meta.json` naming a segment file the copy never got
    /// — and a half-written index was visible at that path throughout.
    #[test]
    fn branch_copy_of_a_live_index_is_readable() {
        let dir = tempfile::TempDir::new().unwrap();
        let engine =
            TantivyIndexingEngine::new(dir.path().to_path_buf(), 512 * 1024 * 1024).unwrap();

        const DOCS: usize = 40;
        for i in 0..DOCS {
            let n = node(i);
            engine
                .do_index_node_with_plan(&job(&n.id), &n, &plan())
                .unwrap();
        }

        let mut copy_job = job("node-0");
        copy_job.kind = JobKind::BranchCreated;
        copy_job.branch = "feature".to_string();
        copy_job.source_branch = Some("main".to_string());
        engine.do_branch_created(&copy_job).unwrap();

        // Checked before anything opens the copy, since opening it creates a
        // writer (and therefore a lock file) of its own.
        let branch_dir = dir.path().join("t").join("r").join("feature");
        assert!(
            !branch_dir.join(".tantivy-writer.lock").exists(),
            "the source's writer lock file was copied into the branch"
        );

        // Open the copy through a fresh engine so nothing is served from the
        // cache the copy itself populated.
        let reopened =
            TantivyIndexingEngine::new(dir.path().to_path_buf(), 512 * 1024 * 1024).unwrap();
        let copied = reopened.get_or_create_index("t", "r", "feature").unwrap();
        let docs = copied.reader.searcher().num_docs();

        assert_eq!(
            docs, DOCS as u64,
            "branch copy has {docs} documents, source had {DOCS}"
        );
    }

    /// Regression: indexing one node at a time must not leave one segment per
    /// commit behind forever.
    ///
    /// Each operation used to build its own `IndexWriter`, and
    /// `IndexWriter::drop` kills the segment updater — so the merge that each
    /// commit scheduled was cancelled before it could produce anything, and
    /// the segment count only ever grew. In production that reached 753
    /// segments for 128k documents and pinned ~2.7 cores in `IndexMerger`.
    ///
    /// With one writer held per index the merges complete, so a run of
    /// single-node commits well past `LogMergePolicy`'s 8-segment floor
    /// settles back down.
    #[test]
    fn single_node_commits_do_not_accumulate_segments() {
        let dir = tempfile::TempDir::new().unwrap();
        let engine =
            TantivyIndexingEngine::new(dir.path().to_path_buf(), 512 * 1024 * 1024).unwrap();

        const COMMITS: usize = 40;
        // Observed settled count is 5 (LogMergePolicy's default 8-segment
        // floor, minus what the last merges absorbed). The bound is loose
        // enough not to encode the policy's exact arithmetic, and far enough
        // below `COMMITS` to fail loudly if merges stop landing at all.
        const SETTLED_MAX: usize = 10;
        for i in 0..COMMITS {
            let n = node(i);
            engine
                .do_index_node_with_plan(&job(&n.id), &n, &plan())
                .unwrap();
        }

        // Merges run on their own threads, so give them a bounded window to
        // land rather than asserting on a race.
        let cached = engine.get_or_create_index("t", "r", "main").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut segments = usize::MAX;
        while std::time::Instant::now() < deadline {
            segments = cached.index.searchable_segment_ids().unwrap().len();
            if segments <= SETTLED_MAX {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        assert!(
            segments <= SETTLED_MAX,
            "{COMMITS} single-node commits left {segments} segments — merges are being \
             cancelled, which means a writer is being dropped per operation again"
        );
    }
}
