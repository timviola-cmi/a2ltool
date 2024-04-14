use crate::dwarf::{DebugData, DwarfDataType, TypeInfo};
use a2lfile::{A2lObject, AddrType, AddressType, Instance, MatrixDim, Module};
use std::collections::HashSet;

use crate::update::{
    cleanup_removed_axis_pts, cleanup_removed_blobs, cleanup_removed_characteristics,
    cleanup_removed_measurements, get_symbol_info,
    ifdata_update::{update_ifdata, zero_if_data},
    log_update_errors, set_symbol_link, TypedefNames, TypedefReferrer, TypedefsRefInfo,
};

pub(crate) fn update_module_instances<'a>(
    module: &mut Module,
    debug_data: &'a DebugData,
    log_msgs: &mut Vec<String>,
    preserve_unknown: bool,
    nameset: &TypedefNames,
) -> (u32, u32, TypedefsRefInfo<'a>) {
    let mut removed_items = HashSet::<String>::new();
    let mut instance_list = Vec::new();
    let mut instance_updated: u32 = 0;
    let mut instance_not_updated: u32 = 0;
    let mut typedef_types = TypedefsRefInfo::new();
    std::mem::swap(&mut module.instance, &mut instance_list);
    for mut instance in instance_list {
        match update_instance_address(&mut instance, debug_data) {
            Ok((typedef_ref, typeinfo)) => {
                if nameset.contains(&typedef_ref) {
                    // Each INSTANCE can have:
                    // - an ADDRESS_TYPE, which means that it is a pointer to some data
                    // - a MATRIX_DIM, meaning this instance is an array of some data
                    // when ADDRESS_TYPE and MATRIX_DIM are present at the same time, the INSTANCE represents
                    // a pointer to an array, not an array of pointers.
                    //
                    // Therefore the typeinfo should be transformed to the base data type by first unwrapping
                    // one pointer (if any), and then getting an array element type (if any)
                    // More complicted constructions like pointers to pointers, arrays of pointers, etc. can not be represented directly
                    let basetype = typeinfo
                        .get_pointer(&debug_data.types)
                        .map_or(typeinfo, |(_, t)| t);
                    let basetype = basetype.get_arraytype().unwrap_or(basetype);

                    typedef_types.entry(typedef_ref).or_default().push((
                        Some(basetype),
                        TypedefReferrer::Instance(module.instance.len()),
                    ));

                    module.instance.push(instance);
                    instance_updated += 1;
                } else if preserve_unknown {
                    module.instance.push(instance);
                    instance_updated += 1;
                } else {
                    log_msgs.push(format!("Error updating INSTANCE on line {}: type ref {} does not refer to any TYPEDEF_*", instance.get_line(), instance.type_ref));
                    instance_not_updated += 1;
                }
            }
            Err(errmsgs) => {
                log_update_errors(log_msgs, errmsgs, "INSTANCE", instance.get_line());

                if preserve_unknown {
                    instance.start_address = 0;
                    zero_if_data(&mut instance.if_data);
                    typedef_types
                        .entry(instance.type_ref.clone())
                        .or_default()
                        .push((None, TypedefReferrer::Instance(module.instance.len())));
                    module.instance.push(instance);
                } else {
                    // item is removed implicitly, because it is not added back to the list
                    removed_items.insert(instance.name.clone());
                }
                instance_not_updated += 1;
            }
        }
    }
    cleanup_removed_instances(module, &removed_items);

    (instance_updated, instance_not_updated, typedef_types)
}

// update the address of an INSTANCE object
fn update_instance_address<'a>(
    instance: &mut Instance,
    debug_data: &'a DebugData,
) -> Result<(String, &'a TypeInfo), Vec<String>> {
    match get_symbol_info(
        &instance.name,
        &instance.symbol_link,
        &instance.if_data,
        debug_data,
    ) {
        Ok(sym_info) => {
            // make sure a valid SYMBOL_LINK exists
            set_symbol_link(&mut instance.symbol_link, sym_info.name.clone());
            instance.start_address = sym_info.address as u32;

            let typeinfo = if let Some((pt_size, basetype)) =
                &sym_info.typeinfo.get_pointer(&debug_data.types)
            {
                let address_type = instance
                    .address_type
                    .get_or_insert(AddressType::new(AddrType::Pbyte));
                match pt_size {
                    1 => address_type.address_type = AddrType::Pbyte,
                    2 => address_type.address_type = AddrType::Pword,
                    4 => address_type.address_type = AddrType::Plong,
                    8 => address_type.address_type = AddrType::Plonglong,
                    _ => instance.address_type = None,
                }
                basetype
            } else {
                instance.address_type = None;
                sym_info.typeinfo
            };

            let typeinfo = if let DwarfDataType::Array { dim, arraytype, .. } = &typeinfo.datatype {
                let matrix_dim = instance.matrix_dim.get_or_insert(MatrixDim::new());
                matrix_dim.dim_list = dim.iter().map(|d| *d as u16).collect();
                update_ifdata(
                    &mut instance.if_data,
                    &sym_info.name,
                    arraytype,
                    sym_info.address,
                );
                arraytype
            } else {
                instance.matrix_dim = None;
                typeinfo
            };

            update_ifdata(
                &mut instance.if_data,
                &sym_info.name,
                typeinfo,
                sym_info.address,
            );

            // return the name of the linked TYPEDEF_<x>
            Ok((instance.type_ref.clone(), sym_info.typeinfo))
        }
        Err(errmsgs) => Err(errmsgs),
    }
}

pub(crate) fn cleanup_removed_instances(module: &mut Module, removed_items: &HashSet<String>) {
    // INSTANCEs can take the place of AXIS_PTS, BLOBs, CHARACTERISTICs or MEASUREMENTs, depending on which kind of TYPEDEF the instance is based on
    cleanup_removed_axis_pts(module, removed_items);
    cleanup_removed_blobs(module, removed_items);
    cleanup_removed_characteristics(module, removed_items);
    cleanup_removed_measurements(module, removed_items);
}
