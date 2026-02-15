//! Dependency Resolution
//!
//! Implements SAT-based dependency resolution for package management.
//! Uses a DPLL solver with unit propagation, pure literal elimination,
//! and backtracking for conflict resolution.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec,
    vec::Vec,
};

use super::{Dependency, PackageId, PackageMetadata, Version};

// ============================================================================
// Version Requirement
// ============================================================================

/// Version requirement supporting exact, range, caret, and tilde expressions
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionReq {
    /// Exact version
    Exact(Version),
    /// Minimum version (>=)
    AtLeast(Version),
    /// Maximum version (<=)
    AtMost(Version),
    /// Range (>= min, < max)
    Range(Version, Version),
    /// Caret range: ^1.2.3 means >=1.2.3, <2.0.0
    Caret(Version),
    /// Tilde range: ~1.2.3 means >=1.2.3, <1.3.0
    Tilde(Version),
    /// Any version
    Any,
    /// Compound requirement: all sub-requirements must be satisfied
    Compound(Vec<VersionReq>),
}

impl VersionReq {
    /// Parse version requirement from string.
    ///
    /// Supports:
    /// - `*` or empty: any version
    /// - `1.2.3`: exact version
    /// - `>=1.2.3`: minimum version
    /// - `<=1.2.3`: maximum version
    /// - `^1.2.3`: compatible with (same major for major > 0)
    /// - `~1.2.3`: approximately (same major.minor)
    /// - `>=1.2.0, <2.0.0`: compound range (comma-separated)
    pub fn parse(s: &str) -> Self {
        let trimmed = s.trim();
        if trimmed == "*" || trimmed.is_empty() {
            return VersionReq::Any;
        }

        // Check for compound expressions (comma-separated)
        if trimmed.contains(',') {
            let parts: Vec<VersionReq> = trimmed
                .split(',')
                .map(|part| Self::parse_single(part.trim()))
                .collect();
            if parts.len() == 1 {
                return parts.into_iter().next().unwrap_or(VersionReq::Any);
            }
            return VersionReq::Compound(parts);
        }

        Self::parse_single(trimmed)
    }

    /// Parse a single (non-compound) version requirement
    fn parse_single(s: &str) -> Self {
        let trimmed = s.trim();

        if let Some(rest) = trimmed.strip_prefix('^') {
            if let Some(v) = Self::parse_version_flexible(rest) {
                return VersionReq::Caret(v);
            }
        }

        if let Some(rest) = trimmed.strip_prefix('~') {
            if let Some(v) = Self::parse_version_flexible(rest) {
                return VersionReq::Tilde(v);
            }
        }

        if let Some(rest) = trimmed.strip_prefix(">=") {
            if let Some(v) = Self::parse_version(rest) {
                return VersionReq::AtLeast(v);
            }
        }

        if let Some(rest) = trimmed.strip_prefix("<=") {
            if let Some(v) = Self::parse_version(rest) {
                return VersionReq::AtMost(v);
            }
        }

        if let Some(rest) = trimmed.strip_prefix('<') {
            // Strict less-than: convert to range 0.0.0..<version
            if let Some(v) = Self::parse_version(rest) {
                return VersionReq::Range(Version::new(0, 0, 0), v);
            }
        }

        if let Some(rest) = trimmed.strip_prefix('>') {
            // Strict greater-than: >=next_patch
            if let Some(v) = Self::parse_version(rest) {
                let next = Version::new(v.major, v.minor, v.patch + 1);
                return VersionReq::AtLeast(next);
            }
        }

        if let Some(v) = Self::parse_version(trimmed) {
            return VersionReq::Exact(v);
        }

        VersionReq::Any
    }

    /// Parse version from string (major.minor.patch)
    fn parse_version(s: &str) -> Option<Version> {
        let parts: Vec<&str> = s.trim().split('.').collect();
        if parts.len() != 3 {
            return None;
        }

        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts[2].parse().ok()?;

        Some(Version::new(major, minor, patch))
    }

    /// Parse version flexibly: `1` -> `1.0.0`, `1.2` -> `1.2.0`, `1.2.3` ->
    /// `1.2.3`
    fn parse_version_flexible(s: &str) -> Option<Version> {
        let parts: Vec<&str> = s.trim().split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }

        let major: u32 = parts[0].parse().ok()?;
        let minor: u32 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch: u32 = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);

        Some(Version::new(major, minor, patch))
    }

    /// Check if version satisfies this requirement
    pub fn satisfies(&self, version: &Version) -> bool {
        match self {
            VersionReq::Exact(v) => version == v,
            VersionReq::AtLeast(v) => version >= v,
            VersionReq::AtMost(v) => version <= v,
            VersionReq::Range(min, max) => version >= min && version < max,
            VersionReq::Caret(v) => satisfies_caret(version, v),
            VersionReq::Tilde(v) => satisfies_tilde(version, v),
            VersionReq::Any => true,
            VersionReq::Compound(reqs) => reqs.iter().all(|r| r.satisfies(version)),
        }
    }
}

/// Check if `version` satisfies caret requirement `^base`.
///
/// - `^1.2.3` means `>=1.2.3, <2.0.0` (major > 0)
/// - `^0.2.3` means `>=0.2.3, <0.3.0` (major == 0, minor > 0)
/// - `^0.0.3` means `>=0.0.3, <0.0.4` (major == 0, minor == 0)
fn satisfies_caret(version: &Version, base: &Version) -> bool {
    if version < base {
        return false;
    }
    if base.major > 0 {
        version.major == base.major
    } else if base.minor > 0 {
        version.major == 0 && version.minor == base.minor
    } else {
        version.major == 0 && version.minor == 0 && version.patch == base.patch
    }
}

/// Check if `version` satisfies tilde requirement `~base`.
///
/// - `~1.2.3` means `>=1.2.3, <1.3.0`
fn satisfies_tilde(version: &Version, base: &Version) -> bool {
    version >= base && version.major == base.major && version.minor == base.minor
}

/// Check if a version satisfies a range expression string.
///
/// Convenience function that parses and evaluates in one step.
pub fn satisfies_range(version: &Version, range_expr: &str) -> bool {
    VersionReq::parse(range_expr).satisfies(version)
}

// ============================================================================
// SAT Solver Types
// ============================================================================

/// A literal is a SAT variable with polarity (positive = include, negative =
/// exclude). Variables are identified by 1-based integer indices.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Literal {
    /// Variable index (1-based)
    var_index: usize,
    /// True if positive (package is selected), false if negated
    positive: bool,
}

impl Literal {
    fn new(var_index: usize, positive: bool) -> Self {
        Self {
            var_index,
            positive,
        }
    }
}

/// A CNF clause is a disjunction (OR) of literals.
#[derive(Debug, Clone)]
struct SatClause {
    literals: Vec<Literal>,
}

impl SatClause {
    fn new(literals: Vec<Literal>) -> Self {
        Self { literals }
    }
}

/// Assignment state for a variable during DPLL solving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Assignment {
    /// Variable is assigned true
    True,
    /// Variable is assigned false
    False,
    /// Variable is unassigned
    Unassigned,
}

/// DPLL-based SAT solver for dependency resolution.
struct SatSolver {
    /// Number of variables
    num_vars: usize,
    /// CNF clauses
    clauses: Vec<SatClause>,
    /// Current variable assignments (index 0 is unused, 1-based)
    assignments: Vec<Assignment>,
}

impl SatSolver {
    fn new(num_vars: usize) -> Self {
        Self {
            num_vars,
            clauses: Vec::new(),
            assignments: vec![Assignment::Unassigned; num_vars + 1],
        }
    }

    fn add_clause(&mut self, clause: SatClause) {
        self.clauses.push(clause);
    }

    /// Run DPLL solving algorithm.
    ///
    /// Returns `Some(assignments)` if satisfiable, `None` if not.
    fn solve(&mut self) -> Option<Vec<bool>> {
        // Reset assignments
        self.assignments = vec![Assignment::Unassigned; self.num_vars + 1];
        if self.dpll() {
            let result = (1..=self.num_vars)
                .map(|i| self.assignments[i] == Assignment::True)
                .collect();
            Some(result)
        } else {
            None
        }
    }

    /// Core DPLL algorithm with unit propagation and pure literal elimination.
    fn dpll(&mut self) -> bool {
        // Unit propagation
        if !self.unit_propagate() {
            return false;
        }

        // Pure literal elimination
        self.pure_literal_eliminate();

        // Check if all clauses are satisfied
        if self.all_clauses_satisfied() {
            return true;
        }

        // Check for empty clause (conflict)
        if self.has_empty_clause() {
            return false;
        }

        // Choose an unassigned variable (first unassigned, prefer lower index)
        let var = match self.choose_variable() {
            Some(v) => v,
            None => return self.all_clauses_satisfied(),
        };

        // Try assigning true first (prefer selecting the package)
        let saved = self.assignments.clone();
        self.assignments[var] = Assignment::True;
        if self.dpll() {
            return true;
        }

        // Backtrack and try false
        self.assignments = saved;
        self.assignments[var] = Assignment::False;
        self.dpll()
    }

    /// Unit propagation: if a clause has only one unassigned literal (and all
    /// others are false), force that literal's value.
    ///
    /// Returns false if a conflict is detected (empty clause under current
    /// assignments).
    fn unit_propagate(&mut self) -> bool {
        let mut changed = true;
        while changed {
            changed = false;
            for clause_idx in 0..self.clauses.len() {
                let mut unassigned_lit: Option<Literal> = None;
                let mut unassigned_count = 0;
                let mut clause_satisfied = false;

                for lit in &self.clauses[clause_idx].literals {
                    match self.assignments[lit.var_index] {
                        Assignment::True => {
                            if lit.positive {
                                clause_satisfied = true;
                                break;
                            }
                        }
                        Assignment::False => {
                            if !lit.positive {
                                clause_satisfied = true;
                                break;
                            }
                        }
                        Assignment::Unassigned => {
                            unassigned_count += 1;
                            unassigned_lit = Some(lit.clone());
                        }
                    }
                }

                if clause_satisfied {
                    continue;
                }

                if unassigned_count == 0 {
                    // All literals are falsified -- conflict
                    return false;
                }

                if unassigned_count == 1 {
                    // Unit clause -- force the remaining literal
                    if let Some(lit) = unassigned_lit {
                        self.assignments[lit.var_index] = if lit.positive {
                            Assignment::True
                        } else {
                            Assignment::False
                        };
                        changed = true;
                    }
                }
            }
        }
        true
    }

    /// Pure literal elimination: if a variable appears only positively (or only
    /// negatively) across all unsatisfied clauses, assign it accordingly.
    fn pure_literal_eliminate(&mut self) {
        let mut appears_pos = vec![false; self.num_vars + 1];
        let mut appears_neg = vec![false; self.num_vars + 1];

        for clause in &self.clauses {
            // Skip already-satisfied clauses
            if self.clause_satisfied(clause) {
                continue;
            }
            for lit in &clause.literals {
                if self.assignments[lit.var_index] == Assignment::Unassigned {
                    if lit.positive {
                        appears_pos[lit.var_index] = true;
                    } else {
                        appears_neg[lit.var_index] = true;
                    }
                }
            }
        }

        for var in 1..=self.num_vars {
            if self.assignments[var] != Assignment::Unassigned {
                continue;
            }
            if appears_pos[var] && !appears_neg[var] {
                self.assignments[var] = Assignment::True;
            } else if !appears_pos[var] && appears_neg[var] {
                self.assignments[var] = Assignment::False;
            }
        }
    }

    /// Check if a single clause is satisfied under current assignments.
    fn clause_satisfied(&self, clause: &SatClause) -> bool {
        clause
            .literals
            .iter()
            .any(|lit| match self.assignments[lit.var_index] {
                Assignment::True => lit.positive,
                Assignment::False => !lit.positive,
                Assignment::Unassigned => false,
            })
    }

    /// Check if all clauses are satisfied under current assignments.
    fn all_clauses_satisfied(&self) -> bool {
        self.clauses.iter().all(|c| self.clause_satisfied(c))
    }

    /// Check if any clause is falsified (all its literals are assigned false).
    fn has_empty_clause(&self) -> bool {
        for clause in &self.clauses {
            let all_falsified =
                clause
                    .literals
                    .iter()
                    .all(|lit| match self.assignments[lit.var_index] {
                        Assignment::True => !lit.positive,
                        Assignment::False => lit.positive,
                        Assignment::Unassigned => false,
                    });
            if all_falsified && !clause.literals.is_empty() {
                return true;
            }
        }
        false
    }

    /// Choose an unassigned variable for branching.
    fn choose_variable(&self) -> Option<usize> {
        (1..=self.num_vars).find(|&i| self.assignments[i] == Assignment::Unassigned)
    }
}

// ============================================================================
// Package Resolution Candidate
// ============================================================================

/// Package resolution candidate
///
/// Phase 4 (package ecosystem) -- used by the SAT resolver internally.
/// Fields `package_id`, `version`, and `provides` are stored for metadata
/// lookup and virtual-package resolution during dependency encoding.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Candidate {
    package_id: PackageId,
    version: Version,
    dependencies: Vec<Dependency>,
    conflicts: Vec<PackageId>,
    /// Virtual packages this candidate provides (e.g., "mail-server")
    provides: Vec<PackageId>,
}

// ============================================================================
// Dependency Resolver
// ============================================================================

/// Dependency resolver with DPLL-based SAT solving.
///
/// Supports:
/// - Greedy resolution (fast path for simple dependency trees)
/// - SAT-based resolution for complex conflict scenarios
/// - Virtual packages via `provides`
/// - Upgrade computation
pub struct DependencyResolver {
    /// Available packages
    available: BTreeMap<PackageId, Vec<Version>>,
    /// Package metadata (dependencies, conflicts, provides)
    metadata: BTreeMap<(PackageId, Version), Candidate>,
    /// Virtual package providers: virtual_name -> [(real_pkg, version)]
    virtual_providers: BTreeMap<PackageId, Vec<(PackageId, Version)>>,
}

impl DependencyResolver {
    pub fn new() -> Self {
        Self {
            available: BTreeMap::new(),
            metadata: BTreeMap::new(),
            virtual_providers: BTreeMap::new(),
        }
    }

    /// Register a package version
    pub fn register_package(
        &mut self,
        package_id: PackageId,
        version: Version,
        dependencies: Vec<Dependency>,
        conflicts: Vec<PackageId>,
    ) {
        self.register_package_full(package_id, version, dependencies, conflicts, Vec::new());
    }

    /// Register a package version with virtual provides
    pub fn register_package_full(
        &mut self,
        package_id: PackageId,
        version: Version,
        dependencies: Vec<Dependency>,
        conflicts: Vec<PackageId>,
        provides: Vec<PackageId>,
    ) {
        // Add to available versions
        self.available
            .entry(package_id.clone())
            .or_insert_with(Vec::new)
            .push(version.clone());

        // Sort versions in descending order (prefer newer)
        if let Some(versions) = self.available.get_mut(&package_id) {
            versions.sort_by(|a, b| b.cmp(a));
        }

        // Register virtual provides
        for virt in &provides {
            self.virtual_providers
                .entry(virt.clone())
                .or_insert_with(Vec::new)
                .push((package_id.clone(), version.clone()));
        }

        // Store metadata
        self.metadata.insert(
            (package_id.clone(), version.clone()),
            Candidate {
                package_id,
                version,
                dependencies,
                conflicts,
                provides,
            },
        );
    }

    /// Resolve dependencies for a package using greedy algorithm.
    ///
    /// Returns a topologically sorted list of packages to install.
    /// Falls back to SAT solving on conflict.
    pub fn resolve(
        &self,
        dependencies: &[Dependency],
    ) -> Result<Vec<(PackageId, Version)>, String> {
        // Try greedy resolution first (fast path)
        match self.resolve_greedy(dependencies) {
            Ok(solution) => Ok(solution),
            Err(_) => {
                // Fall back to SAT-based resolution on conflict
                self.resolve_sat(dependencies)
            }
        }
    }

    /// Greedy dependency resolution (original algorithm).
    fn resolve_greedy(
        &self,
        dependencies: &[Dependency],
    ) -> Result<Vec<(PackageId, Version)>, String> {
        let mut solution = BTreeMap::new();
        let mut visited = BTreeSet::new();

        for dep in dependencies {
            self.resolve_dependency(dep, &mut solution, &mut visited)?;
        }

        self.check_conflicts(&solution)?;

        let mut result: Vec<(PackageId, Version)> = solution.into_iter().collect();
        result.reverse();

        Ok(result)
    }

    /// SAT-based dependency resolution using DPLL solver.
    ///
    /// Encodes the dependency problem as a boolean satisfiability problem
    /// in conjunctive normal form (CNF) and solves it.
    fn resolve_sat(
        &self,
        dependencies: &[Dependency],
    ) -> Result<Vec<(PackageId, Version)>, String> {
        // Collect all relevant packages and versions
        let mut relevant = BTreeSet::new();
        self.collect_relevant_packages(dependencies, &mut relevant)?;

        if relevant.is_empty() {
            return Ok(Vec::new());
        }

        // Assign variable indices to each (package, version) pair
        let mut var_map: BTreeMap<(PackageId, Version), usize> = BTreeMap::new();
        let mut var_list: Vec<(PackageId, Version)> = Vec::new();
        let mut idx = 1usize;
        for (pkg, ver) in &relevant {
            var_map.insert((pkg.clone(), ver.clone()), idx);
            var_list.push((pkg.clone(), ver.clone()));
            idx += 1;
        }
        let num_vars = var_list.len();

        let mut solver = SatSolver::new(num_vars);

        // Encode constraints as CNF clauses
        self.encode_dependencies(dependencies, &var_map, &relevant, &mut solver)?;
        self.encode_at_most_one_version(&var_map, &mut solver);
        self.encode_conflicts(&var_map, &mut solver);

        // Solve
        let solution = solver
            .solve()
            .ok_or_else(|| String::from("No satisfying assignment found for dependencies"))?;

        // Decode solution
        self.decode_solution(&solution, &var_list)
    }

    /// Collect all packages transitively reachable from the given dependencies.
    fn collect_relevant_packages(
        &self,
        dependencies: &[Dependency],
        relevant: &mut BTreeSet<(PackageId, Version)>,
    ) -> Result<(), String> {
        let mut work_queue: Vec<PackageId> = dependencies.iter().map(|d| d.name.clone()).collect();
        let mut visited_pkgs = BTreeSet::new();

        while let Some(pkg) = work_queue.pop() {
            if visited_pkgs.contains(&pkg) {
                continue;
            }
            visited_pkgs.insert(pkg.clone());

            // Resolve virtual packages
            let real_pkg = if self.available.contains_key(&pkg) {
                pkg.clone()
            } else if let Some(providers) = self.virtual_providers.get(&pkg) {
                if let Some((real, _)) = providers.first() {
                    real.clone()
                } else {
                    return Err(alloc::format!("No provider for virtual package: {}", pkg));
                }
            } else {
                return Err(alloc::format!("Package not found: {}", pkg));
            };

            if let Some(versions) = self.available.get(&real_pkg) {
                for ver in versions {
                    relevant.insert((real_pkg.clone(), ver.clone()));
                    // Add transitive dependencies
                    if let Some(candidate) = self.metadata.get(&(real_pkg.clone(), ver.clone())) {
                        for dep in &candidate.dependencies {
                            work_queue.push(dep.name.clone());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Encode dependency constraints as CNF clauses.
    ///
    /// For each required dependency D with version requirement R:
    ///   at least one version of D satisfying R must be selected.
    fn encode_dependencies(
        &self,
        dependencies: &[Dependency],
        var_map: &BTreeMap<(PackageId, Version), usize>,
        relevant: &BTreeSet<(PackageId, Version)>,
        solver: &mut SatSolver,
    ) -> Result<(), String> {
        // Top-level dependencies: at least one satisfying version must be true
        for dep in dependencies {
            let clause = self.build_dependency_clause(dep, var_map)?;
            if clause.literals.is_empty() {
                return Err(alloc::format!(
                    "No version satisfies requirement {} {}",
                    dep.name,
                    dep.version_req
                ));
            }
            solver.add_clause(clause);
        }

        // Transitive dependencies: if package@version is selected,
        // then its dependencies must also be satisfied.
        // Encoding: NOT(pkg@ver) OR dep_ver1 OR dep_ver2 OR ...
        for (pkg, ver) in relevant {
            if let Some(&var_idx) = var_map.get(&(pkg.clone(), ver.clone())) {
                if let Some(candidate) = self.metadata.get(&(pkg.clone(), ver.clone())) {
                    for dep in &candidate.dependencies {
                        let mut lits = vec![Literal::new(var_idx, false)];

                        let dep_pkg = self.resolve_virtual(&dep.name);
                        let req = VersionReq::parse(&dep.version_req);

                        if let Some(versions) = self.available.get(&dep_pkg) {
                            for dep_ver in versions {
                                if req.satisfies(dep_ver) {
                                    if let Some(&dep_var) =
                                        var_map.get(&(dep_pkg.clone(), dep_ver.clone()))
                                    {
                                        lits.push(Literal::new(dep_var, true));
                                    }
                                }
                            }
                        }

                        // If no satisfying version exists, the implication clause
                        // reduces to NOT(pkg@ver) -- package cannot be selected
                        solver.add_clause(SatClause::new(lits));
                    }
                }
            }
        }

        Ok(())
    }

    /// Encode at-most-one-version constraint for each package.
    ///
    /// For each pair of versions (v1, v2) of the same package:
    ///   NOT(pkg@v1) OR NOT(pkg@v2)
    fn encode_at_most_one_version(
        &self,
        var_map: &BTreeMap<(PackageId, Version), usize>,
        solver: &mut SatSolver,
    ) {
        for (pkg, versions) in &self.available {
            let vars: Vec<usize> = versions
                .iter()
                .filter_map(|v| var_map.get(&(pkg.clone(), v.clone())).copied())
                .collect();

            // Pairwise exclusion
            for i in 0..vars.len() {
                for j in (i + 1)..vars.len() {
                    solver.add_clause(SatClause::new(vec![
                        Literal::new(vars[i], false),
                        Literal::new(vars[j], false),
                    ]));
                }
            }
        }
    }

    /// Encode conflict constraints as negative clauses.
    ///
    /// If package A conflicts with package B:
    ///   NOT(A@vN) OR NOT(B@vM) for all version combinations.
    fn encode_conflicts(
        &self,
        var_map: &BTreeMap<(PackageId, Version), usize>,
        solver: &mut SatSolver,
    ) {
        for ((pkg, ver), candidate) in &self.metadata {
            if let Some(&pkg_var) = var_map.get(&(pkg.clone(), ver.clone())) {
                for conflict in &candidate.conflicts {
                    let conflict_pkg = self.resolve_virtual(conflict);
                    if let Some(conflict_versions) = self.available.get(&conflict_pkg) {
                        for cv in conflict_versions {
                            if let Some(&cv_var) = var_map.get(&(conflict_pkg.clone(), cv.clone()))
                            {
                                solver.add_clause(SatClause::new(vec![
                                    Literal::new(pkg_var, false),
                                    Literal::new(cv_var, false),
                                ]));
                            }
                        }
                    }
                }
            }
        }
    }

    /// Build a disjunctive clause for a dependency requirement.
    fn build_dependency_clause(
        &self,
        dep: &Dependency,
        var_map: &BTreeMap<(PackageId, Version), usize>,
    ) -> Result<SatClause, String> {
        let mut lits = Vec::new();
        let req = VersionReq::parse(&dep.version_req);

        let pkg_name = self.resolve_virtual(&dep.name);
        if let Some(versions) = self.available.get(&pkg_name) {
            for ver in versions {
                if req.satisfies(ver) {
                    if let Some(&var_idx) = var_map.get(&(pkg_name.clone(), ver.clone())) {
                        lits.push(Literal::new(var_idx, true));
                    }
                }
            }
        }

        Ok(SatClause::new(lits))
    }

    /// Decode a SAT solution back into a package list.
    fn decode_solution(
        &self,
        solution: &[bool],
        var_list: &[(PackageId, Version)],
    ) -> Result<Vec<(PackageId, Version)>, String> {
        let mut result = Vec::new();

        for (i, selected) in solution.iter().enumerate() {
            if *selected {
                result.push(var_list[i].clone());
            }
        }

        // Topological sort based on dependency edges
        let sorted = self.topological_sort(&result)?;
        Ok(sorted)
    }

    /// Simple topological sort of selected packages based on dependency edges.
    fn topological_sort(
        &self,
        packages: &[(PackageId, Version)],
    ) -> Result<Vec<(PackageId, Version)>, String> {
        let pkg_set: BTreeSet<PackageId> = packages.iter().map(|(p, _)| p.clone()).collect();

        // Build adjacency: dep -> [dependents]
        let mut in_degree: BTreeMap<PackageId, usize> = BTreeMap::new();
        let mut adj: BTreeMap<PackageId, Vec<PackageId>> = BTreeMap::new();

        for (pkg, ver) in packages {
            in_degree.entry(pkg.clone()).or_insert(0);
            adj.entry(pkg.clone()).or_insert_with(Vec::new);

            if let Some(candidate) = self.metadata.get(&(pkg.clone(), ver.clone())) {
                for dep in &candidate.dependencies {
                    let dep_pkg = self.resolve_virtual(&dep.name);
                    if pkg_set.contains(&dep_pkg) {
                        adj.entry(dep_pkg)
                            .or_insert_with(Vec::new)
                            .push(pkg.clone());
                        *in_degree.entry(pkg.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        // Kahn's algorithm
        let mut queue: Vec<PackageId> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(pkg, _)| pkg.clone())
            .collect();
        queue.sort();

        let mut order = Vec::new();
        while let Some(pkg) = queue.pop() {
            order.push(pkg.clone());
            if let Some(dependents) = adj.get(&pkg) {
                for dep in dependents {
                    if let Some(deg) = in_degree.get_mut(dep) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push(dep.clone());
                            queue.sort();
                        }
                    }
                }
            }
        }

        // Map back to (PackageId, Version)
        let pkg_ver_map: BTreeMap<PackageId, Version> = packages
            .iter()
            .map(|(p, v)| (p.clone(), v.clone()))
            .collect();
        let result: Vec<(PackageId, Version)> = order
            .into_iter()
            .filter_map(|p| pkg_ver_map.get(&p).map(|v| (p, v.clone())))
            .collect();

        Ok(result)
    }

    /// Resolve a package name, returning the real package if it is virtual.
    fn resolve_virtual(&self, name: &PackageId) -> PackageId {
        if self.available.contains_key(name) {
            return name.clone();
        }
        if let Some(providers) = self.virtual_providers.get(name) {
            if let Some((real, _)) = providers.first() {
                return real.clone();
            }
        }
        name.clone()
    }

    /// Resolve an upgrade: given currently installed packages and a set of
    /// packages to upgrade, compute the minimal set of packages to install.
    ///
    /// `installed` maps package names to their currently installed versions.
    /// `upgrade_targets` lists packages to upgrade (empty means upgrade all).
    ///
    /// Returns the list of (package, new_version) pairs that should be
    /// installed to satisfy the upgrade.
    pub fn resolve_upgrade(
        &self,
        installed: &BTreeMap<PackageId, Version>,
        upgrade_targets: &[PackageId],
    ) -> Result<Vec<(PackageId, Version)>, String> {
        // Determine which packages to upgrade
        let targets: Vec<PackageId> = if upgrade_targets.is_empty() {
            installed.keys().cloned().collect()
        } else {
            upgrade_targets.to_vec()
        };

        // Build dependencies: each target needs newest available version,
        // all other installed packages must remain satisfiable.
        let mut deps = Vec::new();

        for target in &targets {
            deps.push(Dependency {
                name: target.clone(),
                version_req: String::from("*"),
            });
        }

        // Add non-target installed packages as exact version constraints
        for (pkg, ver) in installed {
            if !targets.contains(pkg) {
                deps.push(Dependency {
                    name: pkg.clone(),
                    version_req: Self::version_to_string(ver),
                });
            }
        }

        // Resolve the full set
        let solution = self.resolve(&deps)?;

        // Filter to only packages that changed
        let mut upgrades = Vec::new();
        for (pkg, new_ver) in &solution {
            match installed.get(pkg) {
                Some(old_ver) if new_ver > old_ver => {
                    upgrades.push((pkg.clone(), new_ver.clone()));
                }
                None => {
                    // New dependency pulled in by upgrade
                    upgrades.push((pkg.clone(), new_ver.clone()));
                }
                _ => {} // Same or downgrade -- skip
            }
        }

        Ok(upgrades)
    }

    /// Resolve a single dependency recursively (greedy algorithm).
    fn resolve_dependency(
        &self,
        dep: &Dependency,
        solution: &mut BTreeMap<PackageId, Version>,
        visited: &mut BTreeSet<PackageId>,
    ) -> Result<(), String> {
        // Check for circular dependencies
        if visited.contains(&dep.name) {
            return Ok(()); // Already being resolved
        }

        // Resolve virtual packages
        let real_name = self.resolve_virtual(&dep.name);

        // Check if already resolved
        if solution.contains_key(&real_name) {
            let existing_version = &solution[&real_name];
            let req = VersionReq::parse(&dep.version_req);

            if !req.satisfies(existing_version) {
                return Err(alloc::format!(
                    "Version conflict for {}: need {}, have {}",
                    dep.name,
                    dep.version_req,
                    Self::version_to_string(existing_version)
                ));
            }
            return Ok(());
        }

        visited.insert(real_name.clone());

        // Find suitable version
        let version = self.find_suitable_version(&real_name, &dep.version_req)?;

        // Get candidate metadata
        let candidate = self
            .metadata
            .get(&(real_name.clone(), version.clone()))
            .ok_or_else(|| {
                alloc::format!(
                    "Missing metadata for {} {}",
                    real_name,
                    Self::version_to_string(&version)
                )
            })?;

        // Resolve transitive dependencies
        for trans_dep in &candidate.dependencies {
            self.resolve_dependency(trans_dep, solution, visited)?;
        }

        // Add to solution
        solution.insert(real_name.clone(), version);

        visited.remove(&real_name);

        Ok(())
    }

    /// Find a suitable version for a package given version requirement
    fn find_suitable_version(
        &self,
        package_id: &PackageId,
        version_req_str: &str,
    ) -> Result<Version, String> {
        let versions = self
            .available
            .get(package_id)
            .ok_or_else(|| alloc::format!("Package not found: {}", package_id))?;

        let req = VersionReq::parse(version_req_str);

        // Find first (newest) version that satisfies requirement
        for version in versions {
            if req.satisfies(version) {
                return Ok(version.clone());
            }
        }

        Err(alloc::format!(
            "No suitable version found for {} (requirement: {})",
            package_id,
            version_req_str
        ))
    }

    /// Check for conflicts in solution
    fn check_conflicts(&self, solution: &BTreeMap<PackageId, Version>) -> Result<(), String> {
        for (pkg_id, version) in solution {
            if let Some(candidate) = self.metadata.get(&(pkg_id.clone(), version.clone())) {
                for conflict in &candidate.conflicts {
                    let resolved = self.resolve_virtual(conflict);
                    if solution.contains_key(&resolved) {
                        return Err(alloc::format!(
                            "Conflict: {} {} conflicts with {}",
                            pkg_id,
                            Self::version_to_string(version),
                            conflict
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Convert version to string
    fn version_to_string(version: &Version) -> String {
        alloc::format!("{}.{}.{}", version.major, version.minor, version.patch)
    }

    /// Get the latest available version for a package.
    pub fn latest_version(&self, package_id: &str) -> Option<Version> {
        self.available
            .get(package_id)
            .and_then(|versions| versions.first().cloned())
    }

    /// Search available packages by name substring.
    ///
    /// Returns matching (package_id, latest_version) pairs.
    pub fn search(&self, query: &str) -> Vec<(PackageId, Version)> {
        let query_lower = query.to_lowercase();
        self.available
            .iter()
            .filter(|(name, _)| name.to_lowercase().contains(&query_lower))
            .filter_map(|(name, versions)| versions.first().map(|v| (name.clone(), v.clone())))
            .collect()
    }

    /// Get metadata for a package (returns the latest version's metadata).
    pub fn get_package_metadata(&self, package_id: &str) -> Option<PackageMetadata> {
        let versions = self.available.get(package_id)?;
        let latest = versions.first()?;
        let candidate = self
            .metadata
            .get(&(String::from(package_id), latest.clone()))?;

        Some(PackageMetadata {
            name: candidate.package_id.clone(),
            version: candidate.version.clone(),
            author: String::from("unknown"),
            description: String::new(),
            license: String::new(),
            dependencies: candidate.dependencies.clone(),
            conflicts: candidate.conflicts.clone(),
        })
    }
}

impl Default for DependencyResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    fn test_version_req_parsing() {
        let req = VersionReq::parse("1.2.3");
        assert!(matches!(req, VersionReq::Exact(_)));

        let req = VersionReq::parse(">=1.0.0");
        assert!(matches!(req, VersionReq::AtLeast(_)));

        let req = VersionReq::parse("*");
        assert!(matches!(req, VersionReq::Any));
    }

    #[test_case]
    fn test_version_satisfies() {
        let v123 = Version::new(1, 2, 3);
        let v100 = Version::new(1, 0, 0);

        let req = VersionReq::Exact(v123.clone());
        assert!(req.satisfies(&v123));
        assert!(!req.satisfies(&v100));

        let req = VersionReq::AtLeast(v100.clone());
        assert!(req.satisfies(&v123));
        assert!(req.satisfies(&v100));
    }

    #[test_case]
    fn test_simple_resolution() {
        let mut resolver = DependencyResolver::new();

        resolver.register_package(String::from("pkg-a"), Version::new(1, 0, 0), vec![], vec![]);

        let deps = vec![Dependency {
            name: String::from("pkg-a"),
            version_req: String::from("1.0.0"),
        }];

        let result = resolver.resolve(&deps);
        assert!(result.is_ok());
        let packages = result.unwrap();
        assert_eq!(packages.len(), 1);
    }

    #[test_case]
    fn test_caret_version_req() {
        let req = VersionReq::parse("^1.2.3");
        assert!(req.satisfies(&Version::new(1, 2, 3)));
        assert!(req.satisfies(&Version::new(1, 9, 0)));
        assert!(!req.satisfies(&Version::new(2, 0, 0)));
        assert!(!req.satisfies(&Version::new(1, 2, 2)));
    }

    #[test_case]
    fn test_caret_zero_major() {
        let req = VersionReq::parse("^0.2.3");
        assert!(req.satisfies(&Version::new(0, 2, 3)));
        assert!(req.satisfies(&Version::new(0, 2, 9)));
        assert!(!req.satisfies(&Version::new(0, 3, 0)));
        assert!(!req.satisfies(&Version::new(1, 0, 0)));
    }

    #[test_case]
    fn test_tilde_version_req() {
        let req = VersionReq::parse("~1.2.3");
        assert!(req.satisfies(&Version::new(1, 2, 3)));
        assert!(req.satisfies(&Version::new(1, 2, 9)));
        assert!(!req.satisfies(&Version::new(1, 3, 0)));
        assert!(!req.satisfies(&Version::new(2, 0, 0)));
    }

    #[test_case]
    fn test_compound_version_req() {
        let req = VersionReq::parse(">=1.2.0, <2.0.0");
        assert!(req.satisfies(&Version::new(1, 2, 0)));
        assert!(req.satisfies(&Version::new(1, 9, 9)));
        assert!(!req.satisfies(&Version::new(2, 0, 0)));
        assert!(!req.satisfies(&Version::new(1, 1, 9)));
    }

    #[test_case]
    fn test_satisfies_range_function() {
        assert!(satisfies_range(&Version::new(1, 5, 0), "^1.2"));
        assert!(!satisfies_range(&Version::new(2, 0, 0), "^1.2"));
        assert!(satisfies_range(&Version::new(1, 2, 5), "~1.2.3"));
    }

    #[test_case]
    fn test_conflict_detection() {
        let mut resolver = DependencyResolver::new();

        resolver.register_package(
            String::from("pkg-a"),
            Version::new(1, 0, 0),
            vec![],
            vec![String::from("pkg-b")],
        );
        resolver.register_package(String::from("pkg-b"), Version::new(1, 0, 0), vec![], vec![]);

        let deps = vec![
            Dependency {
                name: String::from("pkg-a"),
                version_req: String::from("*"),
            },
            Dependency {
                name: String::from("pkg-b"),
                version_req: String::from("*"),
            },
        ];

        let result = resolver.resolve(&deps);
        assert!(result.is_err());
    }

    #[test_case]
    fn test_transitive_dependencies() {
        let mut resolver = DependencyResolver::new();

        resolver.register_package(
            String::from("app"),
            Version::new(1, 0, 0),
            vec![Dependency {
                name: String::from("lib-a"),
                version_req: String::from(">=1.0.0"),
            }],
            vec![],
        );
        resolver.register_package(
            String::from("lib-a"),
            Version::new(1, 2, 0),
            vec![Dependency {
                name: String::from("lib-b"),
                version_req: String::from("^1.0"),
            }],
            vec![],
        );
        resolver.register_package(String::from("lib-b"), Version::new(1, 1, 0), vec![], vec![]);

        let deps = vec![Dependency {
            name: String::from("app"),
            version_req: String::from("*"),
        }];

        let result = resolver.resolve(&deps);
        assert!(result.is_ok());
        let packages = result.unwrap();
        assert_eq!(packages.len(), 3);
    }

    #[test_case]
    fn test_virtual_package_provides() {
        let mut resolver = DependencyResolver::new();

        // "postfix" provides "mail-server"
        resolver.register_package_full(
            String::from("postfix"),
            Version::new(3, 5, 0),
            vec![],
            vec![],
            vec![String::from("mail-server")],
        );

        // "webapp" depends on "mail-server" (virtual)
        resolver.register_package(
            String::from("webapp"),
            Version::new(1, 0, 0),
            vec![Dependency {
                name: String::from("mail-server"),
                version_req: String::from("*"),
            }],
            vec![],
        );

        let deps = vec![Dependency {
            name: String::from("webapp"),
            version_req: String::from("*"),
        }];

        let result = resolver.resolve(&deps);
        assert!(result.is_ok());
        let packages = result.unwrap();
        // Should include webapp and postfix
        assert_eq!(packages.len(), 2);
        assert!(packages.iter().any(|(p, _)| p == "postfix"));
    }

    #[test_case]
    fn test_resolve_upgrade() {
        let mut resolver = DependencyResolver::new();

        resolver.register_package(String::from("pkg-a"), Version::new(1, 0, 0), vec![], vec![]);
        resolver.register_package(String::from("pkg-a"), Version::new(2, 0, 0), vec![], vec![]);
        resolver.register_package(String::from("pkg-b"), Version::new(1, 0, 0), vec![], vec![]);

        let mut installed = BTreeMap::new();
        installed.insert(String::from("pkg-a"), Version::new(1, 0, 0));
        installed.insert(String::from("pkg-b"), Version::new(1, 0, 0));

        let upgrades = resolver
            .resolve_upgrade(&installed, &[String::from("pkg-a")])
            .unwrap();

        assert_eq!(upgrades.len(), 1);
        assert_eq!(upgrades[0].0, "pkg-a");
        assert_eq!(upgrades[0].1, Version::new(2, 0, 0));
    }

    #[test_case]
    fn test_sat_conflict_resolution() {
        let mut resolver = DependencyResolver::new();

        // pkg-a@1 depends on lib@1
        resolver.register_package(
            String::from("pkg-a"),
            Version::new(1, 0, 0),
            vec![Dependency {
                name: String::from("lib"),
                version_req: String::from("1.0.0"),
            }],
            vec![],
        );
        // pkg-a@2 depends on lib@2
        resolver.register_package(
            String::from("pkg-a"),
            Version::new(2, 0, 0),
            vec![Dependency {
                name: String::from("lib"),
                version_req: String::from("2.0.0"),
            }],
            vec![],
        );
        resolver.register_package(String::from("lib"), Version::new(1, 0, 0), vec![], vec![]);
        resolver.register_package(String::from("lib"), Version::new(2, 0, 0), vec![], vec![]);

        // Request pkg-a (any version) -- SAT solver should pick a consistent
        // set
        let deps = vec![Dependency {
            name: String::from("pkg-a"),
            version_req: String::from("*"),
        }];

        let result = resolver.resolve(&deps);
        assert!(result.is_ok());
        let packages = result.unwrap();
        assert_eq!(packages.len(), 2);

        // Verify consistency: pkg-a version matches lib version
        let pkg_a_ver = packages.iter().find(|(p, _)| p == "pkg-a").unwrap();
        let lib_ver = packages.iter().find(|(p, _)| p == "lib").unwrap();
        assert_eq!(pkg_a_ver.1.major, lib_ver.1.major);
    }

    #[test_case]
    fn test_flexible_version_parsing() {
        // ^1 should parse as ^1.0.0
        let req = VersionReq::parse("^1");
        assert!(req.satisfies(&Version::new(1, 5, 0)));
        assert!(!req.satisfies(&Version::new(2, 0, 0)));

        // ~1.2 should parse as ~1.2.0
        let req = VersionReq::parse("~1.2");
        assert!(req.satisfies(&Version::new(1, 2, 5)));
        assert!(!req.satisfies(&Version::new(1, 3, 0)));
    }
}
