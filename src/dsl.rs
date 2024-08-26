use crate::data_type::{DataTypeMetadata, DataTypeRegistry};
use crate::functions::{FunctionMetadata, FunctionRegistry};
use crate::treepp::pushable::{Builder, Pushable};
use anyhow::{Error, Result};
use indexmap::IndexMap;

pub struct DSL {
    pub data_type_registry: DataTypeRegistry,
    pub function_registry: FunctionRegistry,
    pub memory: IndexMap<usize, MemoryEntry>,
    pub memory_last_idx: usize,
    pub trace: Vec<TraceEntry>,
    pub num_inputs: Option<usize>,
    pub hint: Vec<MemoryEntry>,
}

#[derive(Clone, Debug)]
pub struct MemoryEntry {
    pub data_type: String,
    pub data: Element,
    pub description: Option<String>,
}

impl Pushable for &MemoryEntry {
    fn bitcoin_script_push(&self, builder: Builder) -> Builder {
        (&self.data).bitcoin_script_push(builder)
    }
}

#[derive(Clone, Debug)]
pub enum Element {
    Num(i32),
    ManyNum(Vec<i32>),
    Str(Vec<u8>),
    ManyStr(Vec<Vec<u8>>),
}

pub enum ElementType {
    Num,
    ManyNum(usize),
    Str,
    ManyStr(usize),
}

impl Element {
    pub fn match_type(&self, element_type: &ElementType) -> bool {
        match (self, element_type) {
            (Element::Num(_), ElementType::Num) => true,
            (Element::ManyNum(v), ElementType::ManyNum(l)) => v.len() == *l,
            (Element::Str(_), ElementType::Str) => true,
            (Element::ManyStr(v), ElementType::ManyStr(l)) => v.len() == *l,
            (_, _) => false,
        }
    }
}

impl ElementType {
    pub fn len(&self) -> usize {
        match self {
            ElementType::Num | ElementType::Str => 1,
            ElementType::ManyNum(v) | ElementType::ManyStr(v) => *v,
        }
    }
}

impl Pushable for &Element {
    fn bitcoin_script_push(&self, mut builder: Builder) -> Builder {
        match self {
            Element::Num(v) => v.bitcoin_script_push(builder),
            Element::ManyNum(v) => {
                for vv in v.iter() {
                    builder = vv.bitcoin_script_push(builder);
                }
                builder
            }
            Element::Str(v) => v.bitcoin_script_push(builder),
            Element::ManyStr(v) => {
                for vv in v.iter() {
                    builder = vv.bitcoin_script_push(builder);
                }
                builder
            }
        }
    }
}

#[derive(Clone)]
pub enum TraceEntry {
    FunctionCall(String, Vec<usize>),
    AllocatedConstant(usize),
}

impl Element {
    pub fn len(&self) -> usize {
        match self {
            Element::Num(_) => 1,
            Element::ManyNum(v) => v.len(),
            Element::Str(_) => 1,
            Element::ManyStr(v) => v.len(),
        }
    }
}

impl MemoryEntry {
    pub fn new(data_type: impl ToString, data: Element) -> Self {
        Self {
            data_type: data_type.to_string(),
            data,
            description: None,
        }
    }

    pub fn new_with_description(
        data_type: impl ToString,
        data: Element,
        description: impl ToString,
    ) -> Self {
        Self {
            data_type: data_type.to_string(),
            data,
            description: Some(description.to_string()),
        }
    }
}

impl DSL {
    pub fn new() -> Self {
        Self {
            data_type_registry: DataTypeRegistry::new(),
            function_registry: FunctionRegistry::new(),
            memory: IndexMap::new(),
            memory_last_idx: 0,
            trace: vec![],
            num_inputs: None,
            hint: vec![],
        }
    }

    pub fn add_data_type(&mut self, name: impl ToString, element_type: ElementType) {
        self.data_type_registry.map.insert(
            name.to_string(),
            DataTypeMetadata {
                element_type,
                ref_only: false,
            },
        );
    }

    pub fn add_ref_only_data_type(&mut self, name: impl ToString, element_type: ElementType) {
        self.data_type_registry.map.insert(
            name.to_string(),
            DataTypeMetadata {
                element_type,
                ref_only: true,
            },
        );
    }

    pub fn add_function(&mut self, name: impl ToString, meta: FunctionMetadata) {
        self.function_registry.map.insert(name.to_string(), meta);
    }

    fn alloc(&mut self, data_type: impl ToString, data: Element) -> Result<usize> {
        let idx = self.memory_last_idx;
        self.memory_last_idx += 1;

        let data_type_metadata = self.data_type_registry.map.get(&data_type.to_string());

        if data_type_metadata.is_none() {
            return Err(Error::msg("The data type has not been registered"));
        }

        let data_type_metadata = data_type_metadata.unwrap();
        if !data.match_type(&data_type_metadata.element_type) {
            return Err(Error::msg("The data does not match the type definitions"));
        }
        if self.memory.get(&idx).is_some() {
            return Err(Error::msg("Memory is corrupted"));
        }
        self.memory.insert(
            idx,
            MemoryEntry {
                data_type: data_type.to_string(),
                data,
                description: None,
            },
        );
        Ok(idx)
    }

    pub fn alloc_constant(&mut self, data_type: impl ToString, data: Element) -> Result<usize> {
        if self.num_inputs.is_none() {
            self.num_inputs = Some(self.memory_last_idx);
        }
        let idx = Self::alloc(self, data_type, data)?;
        self.trace.push(TraceEntry::AllocatedConstant(idx));
        Ok(idx)
    }

    pub fn alloc_input(&mut self, data_type: impl ToString, data: Element) -> Result<usize> {
        if self.num_inputs.is_some() {
            return Err(Error::msg(
                "Inputs can only be allocated before any execution or allocation for constants",
            ));
        }
        Self::alloc(self, data_type, data)
    }

    pub fn get_num(&mut self, idx: usize) -> Result<i32> {
        match self.memory.get(&idx) {
            Some(MemoryEntry {
                data: Element::Num(v),
                ..
            }) => Ok(*v),
            _ => Err(Error::msg(
                "Cannot read the requested data in memory as a number",
            )),
        }
    }

    pub fn get_many_num(&mut self, idx: usize) -> Result<&[i32]> {
        match self.memory.get(&idx) {
            Some(MemoryEntry {
                data: Element::ManyNum(v),
                ..
            }) => Ok(v.as_slice()),
            _ => Err(Error::msg(
                "Cannot read the requested data in memory as an array of numbers",
            )),
        }
    }

    pub fn get_str(&mut self, idx: usize) -> Result<&[u8]> {
        match self.memory.get(&idx) {
            Some(MemoryEntry {
                data: Element::Str(v),
                ..
            }) => Ok(v.as_slice()),
            _ => Err(Error::msg(
                "Cannot read the requested data in memory as a string",
            )),
        }
    }

    pub fn get_many_str(&mut self, idx: usize) -> Result<&[Vec<u8>]> {
        match self.memory.get(&idx) {
            Some(MemoryEntry {
                data: Element::ManyStr(v),
                ..
            }) => Ok(v.as_slice()),
            _ => Err(Error::msg(
                "Cannot read the requested data in memory as a string",
            )),
        }
    }

    pub fn set_name(&mut self, idx: usize, name: impl ToString) -> Result<()> {
        let entry = self.memory.get_mut(&idx);

        if entry.is_none() {
            Err(Error::msg(
                "Cannot set the name of a memory location because it is not present in the memory",
            ))
        } else {
            entry.unwrap().description = Some(name.to_string());
            Ok(())
        }
    }

    pub fn execute(
        &mut self,
        function_name: impl ToString,
        input_idxs: &[usize],
    ) -> Result<Vec<usize>> {
        if self.num_inputs.is_none() {
            self.num_inputs = Some(self.memory_last_idx);
        }

        if self
            .function_registry
            .map
            .get(&function_name.to_string())
            .is_none()
        {
            return Err(Error::msg("The function has not been registered"));
        }

        let function_metadata = self
            .function_registry
            .map
            .get(&function_name.to_string())
            .unwrap();

        if function_metadata.input.len() != input_idxs.len() {
            return Err(Error::msg("The number of inputs does not match"));
        }

        for (input_idx, &input_type) in input_idxs.iter().zip(function_metadata.input.iter()) {
            let stack_entry = self.memory.get_mut(input_idx).unwrap();
            if stack_entry.data_type != input_type {
                return Err(Error::msg("The input data type mismatches"));
            }
        }

        let output_types = function_metadata.output.clone();
        let exec_result = (function_metadata.trace_generator)(self, &input_idxs)?;

        if exec_result.new_elements.len() != output_types.len() {
            return Err(Error::msg("The number of outputs does not match"));
        }

        self.hint.extend(exec_result.new_hints);

        let mut outputs = vec![];
        for (&output_type, entry) in output_types.iter().zip(exec_result.new_elements) {
            if output_type != entry.data_type {
                return Err(Error::msg("The output data type mismatches"));
            }
            let data_type_metadata = self.data_type_registry.map.get(output_type).unwrap();
            if !entry.data.match_type(&data_type_metadata.element_type) {
                return Err(Error::msg(
                    "The output data does not match the type definitions",
                ));
            }

            let idx = self.memory_last_idx;
            self.memory_last_idx += 1;
            self.memory.insert(idx, entry);
            outputs.push(idx);
        }

        self.trace.push(TraceEntry::FunctionCall(
            function_name.to_string(),
            input_idxs.to_vec(),
        ));

        Ok(outputs)
    }
}
