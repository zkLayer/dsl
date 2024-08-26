use crate::dsl::{MemoryEntry, DSL};
use crate::treepp::Script;
use std::collections::HashMap;

pub struct FunctionRegistry {
    pub map: HashMap<String, FunctionMetadata>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

pub struct FunctionMetadata {
    pub trace_generator: fn(&mut DSL, &[usize]) -> Option<FunctionOutput>,
    pub script_generator: fn(&[usize]) -> Script,
    pub input: Vec<&'static str>,
    pub output: Vec<&'static str>,
}

pub struct FunctionOutput {
    pub new_elements: Vec<MemoryEntry>,
    pub new_hints: Vec<MemoryEntry>,
}
