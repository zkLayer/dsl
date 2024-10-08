use crate::dsl::{TraceEntry, DSL};
use crate::script::CompiledProgram;
use crate::stack::Stack;
use crate::treepp::*;
use anyhow::{Error, Result};
use bitcoin::opcodes::Ordinary::{OP_2DROP, OP_DROP, OP_FROMALTSTACK};
use bitcoin::ScriptBuf;
use crate::functions::AcceptableFunctionMetadata;
use crate::options::Options;

pub struct Compiler;

impl Compiler {
    pub fn compiler(dsl: DSL) -> Result<CompiledProgram> {
        // step 1: count the last visit of all the memory entries
        let num_memory_entries = dsl.memory_last_idx;
        let mut last_visit = vec![-1isize; num_memory_entries];

        let mut cur_time = 0;
        for trace_entry in dsl.trace.iter() {
            match trace_entry {
                TraceEntry::FunctionCall(_, inputs) => {
                    for &i in inputs.iter() {
                        last_visit[i] = cur_time;
                    }
                    cur_time += 1;
                }
                _ => {}
            }
        }

        // step 2: allocate all the inputs
        let mut input = vec![];
        if let Some(num_inputs) = dsl.num_inputs {
            for i in 0..num_inputs {
                input.push(dsl.memory.get(&i).unwrap().clone())
            }
        }

        // step 3: initialize the stack
        let mut stack = Stack::new(dsl.memory_last_idx);
        for (i, input_entry) in input.iter().enumerate() {
            stack.push_to_stack(i, input_entry.data.len())?;
        }

        // step 3: generate the script
        let mut script = Vec::<u8>::new();

        let mut cur_time = 0;
        let mut allocated_idx = dsl.num_inputs.unwrap_or_default();

        for trace_entry in dsl.trace.iter() {
            match trace_entry {
                TraceEntry::FunctionCall(function_name, inputs) => {
                    let function_metadata = dsl
                        .function_registry
                        .map
                        .get(&function_name.to_string())
                        .unwrap();

                    let input = match function_metadata {
                        AcceptableFunctionMetadata::FunctionWithoutOptions(v) => &v.input,
                        AcceptableFunctionMetadata::FunctionWithOptions(v) => &v.input
                    };

                    let mut deferred_ref = vec![];
                    let mut num_cloned_input_elements = 0;
                    for (i, (&input_idx, input_type)) in inputs
                        .iter()
                        .zip(input.iter())
                        .enumerate()
                    {
                        let input_type_name = dsl.memory.get(&input_idx).unwrap().data_type.clone();

                        let input_metadata = dsl
                            .data_type_registry
                            .map
                            .get(&input_type_name.to_string())
                            .unwrap();

                        if input_type.starts_with("&") {
                            deferred_ref.push(input_idx);
                            // do not obtain the location of the ref-only element before we clone other inputs.
                        } else {
                            let len = input_metadata.element_type.len();
                            let pos = stack.get_relative_position(input_idx)?;
                            let distance = pos + num_cloned_input_elements;

                            if last_visit[input_idx] == cur_time
                                && !inputs[i..].contains(&input_idx)
                                && !dsl.output.contains(&input_idx)
                            {
                                // roll
                                stack.pull(input_idx)?;
                                script.extend_from_slice(roll_script(distance, len).as_bytes());
                                num_cloned_input_elements += len;
                            } else {
                                // pick
                                script.extend_from_slice(pick_script(distance, len).as_bytes());
                                num_cloned_input_elements += len;
                            }
                        }
                    }

                    // It takes into the account of the elements that disappear due to pull,
                    // but it doesn't consider elements that are just copied/moved near the function stack.
                    let mut ref_positions = vec![];
                    for &input_idx in deferred_ref.iter() {
                        ref_positions.push(stack.get_relative_position(input_idx)?);
                    }

                    match function_metadata {
                        AcceptableFunctionMetadata::FunctionWithoutOptions(v) => {
                            script.extend_from_slice((v.script_generator)(&ref_positions)?.as_bytes());
                        }
                        AcceptableFunctionMetadata::FunctionWithOptions(v) => {
                            script.extend_from_slice((v.script_generator)(&ref_positions, &Options::new())?.as_bytes());
                        }
                    }

                    let output = match function_metadata {
                        AcceptableFunctionMetadata::FunctionWithoutOptions(v) => &v.output,
                        AcceptableFunctionMetadata::FunctionWithOptions(v) => &v.output,
                    };

                    // push the corresponding outputs
                    for output_type in output.iter() {
                        let data_type_metadata = dsl
                            .data_type_registry
                            .map
                            .get(&output_type.to_string())
                            .unwrap();
                        stack
                            .push_to_stack(allocated_idx, data_type_metadata.element_type.len())?;
                        allocated_idx += 1;
                    }

                    cur_time += 1;
                }
                TraceEntry::FunctionCallWithOptions(function_name, inputs, options) => {
                    let function_metadata = dsl
                        .function_registry
                        .map
                        .get(&function_name.to_string())
                        .unwrap();

                    let function_metadata = match function_metadata {
                        AcceptableFunctionMetadata::FunctionWithOptions(v) => v,
                        _ => return Err(Error::msg("The function does not offer options")),
                    };

                    let mut deferred_ref = vec![];
                    let mut num_cloned_input_elements = 0;
                    for (i, (&input_idx, input_type)) in inputs
                        .iter()
                        .zip(function_metadata.input.iter())
                        .enumerate()
                    {
                        let input_type_name = dsl.memory.get(&input_idx).unwrap().data_type.clone();

                        let input_metadata = dsl
                            .data_type_registry
                            .map
                            .get(&input_type_name.to_string())
                            .unwrap();

                        if input_type.starts_with("&") {
                            deferred_ref.push(input_idx);
                            // do not obtain the location of the ref-only element before we clone other inputs.
                        } else {
                            let len = input_metadata.element_type.len();
                            let pos = stack.get_relative_position(input_idx)?;
                            let distance = pos + num_cloned_input_elements;

                            if last_visit[input_idx] == cur_time
                                && !inputs[i..].contains(&input_idx)
                                && !dsl.output.contains(&input_idx)
                            {
                                // roll
                                stack.pull(input_idx)?;
                                script.extend_from_slice(roll_script(distance, len).as_bytes());
                                num_cloned_input_elements += len;
                            } else {
                                // pick
                                script.extend_from_slice(pick_script(distance, len).as_bytes());
                                num_cloned_input_elements += len;
                            }
                        }
                    }

                    // It takes into the account of the elements that disappear due to pull,
                    // but it doesn't consider elements that are just copied/moved near the function stack.
                    let mut ref_positions = vec![];
                    for &input_idx in deferred_ref.iter() {
                        ref_positions.push(stack.get_relative_position(input_idx)?);
                    }

                    script.extend_from_slice((function_metadata.script_generator)(&ref_positions, &options)?.as_bytes());

                    // push the corresponding outputs
                    for output_type in function_metadata.output.iter() {
                        let data_type_metadata = dsl
                            .data_type_registry
                            .map
                            .get(&output_type.to_string())
                            .unwrap();
                        stack
                            .push_to_stack(allocated_idx, data_type_metadata.element_type.len())?;
                        allocated_idx += 1;
                    }

                    cur_time += 1;
                }
                TraceEntry::AllocatedConstant(idx) => {
                    let data_type = &dsl.memory.get(idx).unwrap().data_type;
                    let input_metadata = dsl
                        .data_type_registry
                        .map
                        .get(&data_type.to_string())
                        .unwrap();
                    stack.push_to_stack(*idx, input_metadata.element_type.len())?;
                    allocated_idx += 1;

                    script.extend_from_slice(
                        script! {
                            { dsl.memory.get(idx).unwrap() }
                        }
                        .as_bytes(),
                    );
                }
                TraceEntry::AllocatedHint(idx) => {
                    let data_type = &dsl.memory.get(idx).unwrap().data_type;
                    let input_metadata = dsl
                        .data_type_registry
                        .map
                        .get(&data_type.to_string())
                        .unwrap();
                    let len = input_metadata.element_type.len();
                    stack.push_to_stack(*idx, len)?;
                    allocated_idx += 1;

                    script.extend_from_slice(
                        script! {
                            for _ in 0..len {
                                OP_DEPTH OP_1SUB OP_ROLL
                            }
                        }
                        .as_bytes(),
                    );
                }
            }
        }

        // step 4: move the desired output to the altstack
        let mut output_list_rev = dsl.output.clone();
        output_list_rev.reverse();

        let mut output_total_len = 0;

        for (i, &idx) in output_list_rev.iter().enumerate() {
            // for each entry, roll or pick the data and then save the data to the altstack
            // - roll, if this is the last occurrence of this idx in `output_list_rev`
            // - pick, if this idx may occur another time in the remainder of `output_list_rev`
            //
            // the list is reversed with the mind that doing so may reduce the pull/roll distance and save the script length

            let pos = stack.get_relative_position(idx)?;
            let len = stack.get_length(idx)?;

            if output_list_rev[i..].contains(&idx) {
                // pick
                script.extend_from_slice(
                    script! {
                        for _ in 0..len {
                            { pos } OP_PICK
                        }
                        for _ in 0..len {
                            OP_TOALTSTACK
                        }
                    }
                    .as_bytes(),
                );
            } else {
                // roll
                stack.pull(idx)?;
                script.extend_from_slice(
                    script! {
                        for _ in 0..len {
                            { pos } OP_ROLL
                        }
                        for _ in 0..len {
                            OP_TOALTSTACK
                        }
                    }
                    .as_bytes(),
                );
            }
            output_total_len += len;
        }

        // clear all the remaining elements
        let elements_in_stack = stack.get_num_elements_in_stack()?;
        for _ in 0..elements_in_stack / 2 {
            script.push(OP_2DROP.to_u8());
        }
        if elements_in_stack % 2 == 1 {
            script.push(OP_DROP.to_u8());
        }

        // recover the output from the altstack
        for _ in 0..output_total_len {
            script.push(OP_FROMALTSTACK.to_u8());
        }

        Ok(CompiledProgram {
            input,
            script: ScriptBuf::from_bytes(script),
            hint: dsl.hint,
        })
    }
}

fn roll_script(distance: usize, len: usize) -> Script {
    if distance == len - 1 {
        script! {} // do nothing, it is already on the top of the stack
    } else {
        if distance == 1 {
            script! {
                for _ in 0..len {
                    OP_SWAP
                }
            }
        } else if distance == 2 {
            script! {
                for _ in 0..len {
                    OP_ROT
                }
            }
        } else {
            script! {
                for _ in 0..len {
                    { distance } OP_ROLL
                }
            }
        }
    }
}

fn pick_script(distance: usize, len: usize) -> Script {
    if distance == 0 {
        script! {
            for _ in 0..len {
                OP_DUP
            }
        }
    } else if distance == 1 {
        script! {
            for _ in 0..len {
                OP_OVER
            }
        }
    } else {
        script! {
            for _ in 0..len {
                { distance } OP_PICK
            }
        }
    }
}
