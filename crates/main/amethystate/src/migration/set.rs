use super::MigrationPlan;
use crate::MigrationError;
use crate::migration::fields::FieldDescriptor;
use crate::migration::provided::Provided;
use crate::store::StorageResult;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct MigrationSet {
    migrators: HashMap<String, MigrationPlan>,
    targets: HashMap<String, (u32, u32, &'static [FieldDescriptor])>,
    graph: DiGraph<String, ()>,
    nodes: HashMap<String, NodeIndex>,

    /// What the steps need from outside the store. Carried here because a
    /// step is a bare `fn` with nothing to capture, and because these exist
    /// for the migrations and nothing else.
    provided: Provided,
}

impl MigrationSet {
    /// Hands a value to every step this set runs. See
    /// [`StoreBuilder::provide`](crate::StoreBuilder::provide).
    pub fn provide<T: std::any::Any>(&mut self, value: T) {
        self.provided.insert(value);
    }

    pub(crate) fn take_provided(&mut self, provided: Provided) {
        self.provided = provided;
    }

    pub(crate) fn provided(&self) -> &Provided {
        &self.provided
    }
    pub fn add(
        mut self,
        prefix: impl Into<String>,
        migrator: MigrationPlan,
        hash: u32,
        fields: &'static [FieldDescriptor],
        deps: &[&str],
    ) -> Self {
        let prefix = prefix.into();

        let node_idx = *self
            .nodes
            .entry(prefix.clone())
            .or_insert_with_key(|p| self.graph.add_node(p.clone()));

        for dep in deps {
            let dep_idx = *self
                .nodes
                .entry(dep.to_string())
                .or_insert_with_key(|p| self.graph.add_node(p.clone()));

            self.graph.add_edge(dep_idx, node_idx, ());
        }

        let target_version = migrator
            .steps
            .iter()
            .map(|s| s.target_version())
            .max()
            .unwrap_or(0);

        self.targets
            .insert(prefix.clone(), (target_version, hash, fields));
        self.migrators.insert(prefix, migrator);

        self
    }

    /// The version, hash and fields the code declares for `prefix`.
    ///
    /// A set that was given steps for the prefix knows this from them. One
    /// that was not - a store opened with
    /// [`build`](crate::StoreBuilder::build), which runs only what was
    /// declared by hand - reads it from the schema instead, because the schema
    /// is what the code says its shape is whether or not anyone collected the
    /// steps to get there.
    ///
    /// Answering zero there was how opening with `build` came to report drift
    /// for a reason that had nothing to do with the data: the store held a
    /// version and the code appeared to declare none. A prefix that really is
    /// undeclared still answers zero, because nothing declared it.
    pub(crate) fn get_target(&self, prefix: &str) -> (u32, u32, &'static [FieldDescriptor]) {
        if let Some(target) = self.targets.get(prefix) {
            return target.clone();
        }

        inventory::iter::<crate::observability::SchemaEntry>
            .into_iter()
            .find(|entry| entry.prefix.as_ref().is_some_and(|p| p.as_str() == prefix))
            .map(|entry| (entry.version, entry.schema_hash, entry.fields))
            .unwrap_or((0, 0, &[]))
    }

    pub(crate) fn find_components(&self) -> Vec<Vec<String>> {
        let mut visited = HashSet::new();
        let mut components = Vec::new();
        let mut nodes: Vec<_> = self.graph.node_indices().collect();
        nodes.sort_by_key(|&i| &self.graph[i]);

        for node in nodes {
            if !visited.contains(&node) {
                let mut comp = Vec::new();
                let mut stack = vec![node];
                visited.insert(node);

                while let Some(curr) = stack.pop() {
                    comp.push(self.graph[curr].clone());
                    for n in self.graph.neighbors_undirected(curr) {
                        if visited.insert(n) {
                            stack.push(n);
                        }
                    }
                }
                comp.sort();
                components.push(comp);
            }
        }
        components.sort_by(|a, b| a[0].cmp(&b[0]));
        components
    }

    pub(crate) fn topo_sort_component(&self, prefixes: &[String]) -> StorageResult<Vec<String>> {
        let mut sub_graph = DiGraph::new();
        let mut sub_nodes = HashMap::new();

        for p in prefixes {
            let idx = sub_graph.add_node(p.clone());
            sub_nodes.insert(p, idx);
        }

        for p in prefixes {
            let src_idx = self.nodes[p];
            for edge in self.graph.edges(src_idx) {
                let target_prefix = &self.graph[edge.target()];
                if prefixes.contains(target_prefix) {
                    sub_graph.add_edge(sub_nodes[p], sub_nodes[target_prefix], ());
                }
            }
        }

        toposort(&sub_graph, None)
            .map(|indices| {
                indices
                    .into_iter()
                    .map(|idx| sub_graph[idx].clone())
                    .collect()
            })
            .map_err(|cycle| {
                error_stack::Report::new(MigrationError::Cycle(sub_graph[cycle.node_id()].clone()))
                    .change_context(crate::store::StorageError::Migrate)
            })
    }

    pub(crate) fn get_migration_plan(&self, prefix: &str) -> Option<&MigrationPlan> {
        self.migrators.get(prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::migration::fields::FieldDescriptor;

    const EMPTY_FIELDS: &[FieldDescriptor] = &[];

    fn dummy_migrator() -> MigrationPlan {
        MigrationPlan::new()
    }

    #[test]
    fn test_wcc_separation() {
        let set = MigrationSet::default()
            .add("a", dummy_migrator(), 0, EMPTY_FIELDS, &["b"])
            .add("b", dummy_migrator(), 0, EMPTY_FIELDS, &[])
            .add("c", dummy_migrator(), 0, EMPTY_FIELDS, &["d"])
            .add("d", dummy_migrator(), 0, EMPTY_FIELDS, &[])
            .add("e", dummy_migrator(), 0, EMPTY_FIELDS, &[]);

        let components = set.find_components();

        assert_eq!(components.len(), 3);
        assert_eq!(components[0], vec!["a", "b"]);
        assert_eq!(components[1], vec!["c", "d"]);
        assert_eq!(components[2], vec!["e"]);
    }

    #[test]
    fn test_toposort_simple() {
        let set = MigrationSet::default()
            .add("ui", dummy_migrator(), 0, EMPTY_FIELDS, &["app", "net"])
            .add("app", dummy_migrator(), 0, EMPTY_FIELDS, &["net"])
            .add("net", dummy_migrator(), 0, EMPTY_FIELDS, &[]);

        let comp = &set.find_components()[0];
        let sorted = set.topo_sort_component(comp).unwrap();

        assert_eq!(sorted, vec!["net", "app", "ui"]);
    }

    #[test]
    fn test_diamond_dependency() {
        let set = MigrationSet::default()
            .add("d", dummy_migrator(), 0, EMPTY_FIELDS, &["b", "c"])
            .add("b", dummy_migrator(), 0, EMPTY_FIELDS, &["a"])
            .add("c", dummy_migrator(), 0, EMPTY_FIELDS, &["a"])
            .add("a", dummy_migrator(), 0, EMPTY_FIELDS, &[]);

        let comp = &set.find_components()[0];
        let sorted = set.topo_sort_component(comp).unwrap();

        assert_eq!(sorted[0], "a");
        assert!(sorted[1] == "b" || sorted[1] == "c");
        assert!(sorted[2] == "b" || sorted[2] == "c");
        assert_eq!(sorted[3], "d");
    }

    #[test]
    fn test_cycle_error() {
        let set = MigrationSet::default()
            .add("a", dummy_migrator(), 0, EMPTY_FIELDS, &["b"])
            .add("b", dummy_migrator(), 0, EMPTY_FIELDS, &["c"])
            .add("c", dummy_migrator(), 0, EMPTY_FIELDS, &["a"]);

        let comp = &set.find_components()[0];
        let result = set.topo_sort_component(comp).unwrap_err();

        match result.downcast_ref::<MigrationError>() {
            Some(MigrationError::Cycle(prefix)) => {
                assert!(["a", "b", "c"].contains(&prefix.as_str()));
            }
            _ => panic!("Expected MigrationCycle error, got {:?}", result),
        }
    }

    #[test]
    fn test_target_info_retrieval() {
        static TEST_FIELDS: &[FieldDescriptor] = &[FieldDescriptor::leaf("id", 123, "u64")];

        let migrator = MigrationPlan::new().step(1, "init", |_| Ok(()));
        let set = MigrationSet::default().add("app", migrator, 999, TEST_FIELDS, &[]);

        let (v, h, f) = set.get_target("app");
        assert_eq!(v, 1);
        assert_eq!(h, 999);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "id");
        assert_eq!(f[0].type_name, "u64");
    }

    #[test]
    fn test_implicit_dependencies() {
        let set = MigrationSet::default().add("a", dummy_migrator(), 0, EMPTY_FIELDS, &["b"]);

        let components = set.find_components();
        assert_eq!(components[0], vec!["a", "b"]);

        let sorted = set.topo_sort_component(&components[0]).unwrap();
        assert_eq!(sorted, vec!["b", "a"]);
    }

    #[test]
    fn test_component_determinism() {
        let set1 = MigrationSet::default()
            .add("x", dummy_migrator(), 0, EMPTY_FIELDS, &[])
            .add("a", dummy_migrator(), 0, EMPTY_FIELDS, &[]);

        let set2 = MigrationSet::default()
            .add("a", dummy_migrator(), 0, EMPTY_FIELDS, &[])
            .add("x", dummy_migrator(), 0, EMPTY_FIELDS, &[]);

        assert_eq!(set1.find_components(), set2.find_components());
    }
}
