use std::collections::HashMap;

use crate::model::{Plan, PlanPackage, Unit};

pub(crate) fn units_by_package(plan: &Plan) -> HashMap<String, Vec<Unit>> {
    plan.packages
        .iter()
        .map(|package| {
            let units = plan
                .units
                .iter()
                .filter(|unit| unit.package_key == package.key)
                .cloned()
                .collect::<Vec<_>>();
            (package.key.clone(), units)
        })
        .collect()
}

pub(crate) fn topologically_sorted_packages(plan: &Plan) -> Vec<&PlanPackage> {
    fn visit<'a>(
        index: usize,
        plan: &'a Plan,
        key_to_index: &HashMap<&'a str, usize>,
        marks: &mut [u8],
        out: &mut Vec<&'a PlanPackage>,
    ) {
        if marks[index] == 2 {
            return;
        }
        if marks[index] == 1 {
            return;
        }

        marks[index] = 1;
        for dependency in &plan.packages[index].dependencies {
            if let Some(&dep_index) = key_to_index.get(dependency.as_str()) {
                visit(dep_index, plan, key_to_index, marks, out);
            }
        }
        marks[index] = 2;
        out.push(&plan.packages[index]);
    }

    let key_to_index = plan
        .packages
        .iter()
        .enumerate()
        .map(|(index, package)| (package.key.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut marks = vec![0u8; plan.packages.len()];
    let mut out = Vec::with_capacity(plan.packages.len());

    for index in 0..plan.packages.len() {
        visit(index, plan, &key_to_index, &mut marks, &mut out);
    }

    out
}
