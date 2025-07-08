extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use syn::LitStr;
use syn::{parse_file, ItemFn};

/// generate_configure 是一个过程宏，它会扫描整个项目和 workspace 成员中的路由函数，
/// 然后自动生成 configure 函数来注册这些路由。
///
#[proc_macro]
pub fn generate_configure(_input: TokenStream) -> TokenStream {
    let functions = scan_crate_for_route_functions();

    println!("🔍 Found {} route functions", functions.len());
    for func in &functions {
        println!(
            " - {} [{} {}] (module: {:?})",
            func.name, func.method, func.route_path, func.module_prefix
        );
    }

    let mut grouped: HashMap<Vec<String>, Vec<RouteFunction>> = HashMap::new();
    for func in functions {
        let module_segments: Vec<String> = func
            .module_prefix
            .split("::")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();

        grouped.entry(module_segments).or_default().push(func);
    }

    let mut all_configure_fns = Vec::new();
    let mut all_configure_calls = Vec::new();

    for (module_path, funcs) in grouped {
        let safe_mod_name = module_path.join("_");
        let configure_ident = syn::Ident::new(
            &format!("configure_{}", safe_mod_name),
            proc_macro2::Span::call_site(),
        );

        let scope_name = module_path
            .iter()
            .skip(2)
            .map(|s| s.as_str())
            .collect::<Vec<&str>>()
            .join("/");
        let mod_scope = if scope_name.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", scope_name)
        };

        let services = funcs.iter().map(|f| {
            let ident = syn::Ident::new(&f.name, proc_macro2::Span::call_site());

            let mut segments =
                syn::punctuated::Punctuated::<syn::PathSegment, syn::Token![::]>::new();
            for s in f.module_prefix.split("::") {
                let ident_segment = syn::Ident::new(s, proc_macro2::Span::call_site());
                segments.push(syn::PathSegment::from(ident_segment));
            }

            quote! {
                cfg.service(#segments::#ident);
            }
        });

        let register_ident = syn::Ident::new(
            &format!("register_{}", safe_mod_name),
            proc_macro2::Span::call_site(),
        );

        let register_fn = quote! {
            pub fn #register_ident(cfg: &mut actix_web::web::ServiceConfig) {
                #(#services)*
            }
        };

        let configure_fn = quote! {
            pub fn #configure_ident(cfg: &mut actix_web::web::ServiceConfig) {
                cfg.service(actix_web::web::scope(#mod_scope)
                    .configure(#register_ident));
            }
        };

        all_configure_fns.push(register_fn);
        all_configure_fns.push(configure_fn);
        all_configure_calls.push(configure_ident);
    }

    let expanded = quote! {
        #(#all_configure_fns)*

        pub fn configure(cfg: &mut actix_web::web::ServiceConfig) {
            #(
                cfg.configure(#all_configure_calls);
            )*
        }
    };

    #[cfg(debug_assertions)]
    {
        let generated_code = expanded.to_string();
        println!("🧾 Generated code:\n{}", generated_code);
    }

    TokenStream::from(expanded)
}

fn scan_crate_for_route_functions() -> Vec<RouteFunction> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR environment variable not found");

    let mut result = Vec::new();

    scan_project(&manifest_dir, &mut result);

    if let Some(workspace_config) = read_workspace_config(&manifest_dir) {
        if let Some(members) = workspace_config.members {
            let workspace_dir = PathBuf::from(&manifest_dir);
            scan_workspace_members(workspace_dir, members, &mut result);
        }
    }

    result
}

fn scan_workspace_members(
    workspace_dir: PathBuf,
    members: Vec<String>,
    result: &mut Vec<RouteFunction>,
) {
    for member in members {
        let member_dir = workspace_dir.join(&member);
        if !member_dir.exists() {
            continue;
        }

        let member_manifest_path = member_dir.join("Cargo.toml");
        if !member_manifest_path.exists() {
            continue;
        }

        if let Ok(member_manifest_dir) = member_dir.into_os_string().into_string() {
            scan_project(&member_manifest_dir, result);
        }
    }
}

fn scan_project(manifest_dir: &str, result: &mut Vec<RouteFunction>) {
    let src_path = PathBuf::from(manifest_dir).join("src");

    let main_or_lib_path = match find_main_or_lib(&src_path) {
        Some(path) => path,
        None => return,
    };

    let root_dir = main_or_lib_path.parent().unwrap_or(&src_path);

    let file_name_to_exclude = main_or_lib_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| vec![s, "mod.rs"])
        .unwrap_or_else(|| vec!["mod.rs"]);

    scan_directory(root_dir, &file_name_to_exclude[..], result);
}

#[derive(Debug)]
struct WorkspaceConfig {
    members: Option<Vec<String>>,
}

fn read_workspace_config(manifest_dir: &str) -> Option<WorkspaceConfig> {
    use toml::Value;

    let mut path = PathBuf::from(manifest_dir);
    path.push("Cargo.toml");

    let mut file = fs::File::open(path).ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).ok()?;

    let cargo_toml: HashMap<String, Value> = toml::from_str(&contents).ok()?;
    let workspace_val = cargo_toml.get("workspace")?;
    let members_val = workspace_val.get("members")?;

    if let Some(Value::Array(members)) = Some(members_val) {
        let members_vec: Vec<String> = members
            .iter()
            .filter_map(|member| member.as_str().map(|s| s.to_string()))
            .collect();

        return Some(WorkspaceConfig {
            members: if members_vec.is_empty() {
                None
            } else {
                Some(members_vec)
            },
        });
    }

    None
}

fn find_main_or_lib(src_path: &Path) -> Option<PathBuf> {
    let main_rs = src_path.join("main.rs");
    let lib_rs = src_path.join("lib.rs");

    if main_rs.exists() {
        Some(main_rs)
    } else if lib_rs.exists() {
        Some(lib_rs)
    } else {
        None
    }
}

fn scan_directory<P: AsRef<Path>>(
    path: P,
    exclude_files: &[&str],
    result: &mut Vec<RouteFunction>,
) {
    let path = path.as_ref();

    if let Ok(entries) = fs::read_dir(path) {
        entries
            .filter_map(|entry| entry.ok())
            .par_bridge()
            .filter_map(|entry| {
                let entry_path = entry.path();
                let file_name = entry_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");

                if entry_path.is_file() {
                    let ext = entry_path.extension().and_then(|s| s.to_str());
                    if ext == Some("rs") && !exclude_files.contains(&file_name) {
                        let mut sub_result = Vec::new();
                        process_file(&entry_path, &mut sub_result);
                        return Some(sub_result);
                    }
                } else if entry_path.is_dir() {
                    let mut sub_result = Vec::new();
                    scan_directory(&entry_path, exclude_files, &mut sub_result);
                    return Some(sub_result);
                }

                None
            })
            .flatten()
            .collect::<Vec<_>>()
            .into_iter()
            .for_each(|item| result.push(item));
    } else {
        eprintln!("❌ Failed to read directory: {:?}", path);
    }
}

fn process_file(path: &Path, result: &mut Vec<RouteFunction>) {
    if let Ok(content) = fs::read_to_string(path) {
        scan_file(&content, result, path);
    } else {
        eprintln!("❌ Failed to read file: {}", path.display());
    }
}

fn scan_file(content: &str, result: &mut Vec<RouteFunction>, path: &Path) {
    let file = parse_file(content).expect("Failed to parse file content");

    let mut current_module = vec![];

    if let Some(file_name) = path.file_stem().and_then(|s| s.to_str()) {
        if file_name != "mod" {
            if !file
                .items
                .iter()
                .any(|item| matches!(item, syn::Item::Mod(_)))
            {
                current_module.push(file_name.to_string());
            }
        }
    }

    for item in file.items {
        process_item_with_module(&item, result, &mut current_module, path);
    }
}

fn process_item_with_module(
    item: &syn::Item,
    result: &mut Vec<RouteFunction>,
    current_module: &mut Vec<String>,
    path: &Path,
) {
    match item {
        syn::Item::Fn(fn_item) => handle_function(fn_item, result, current_module),
        syn::Item::Mod(module) => handle_module(module, result, current_module, path),
        _ => {}
    }
}

fn handle_function(
    fn_item: &ItemFn,
    result: &mut Vec<RouteFunction>,
    current_module: &mut Vec<String>,
) {
    let route_fn = match extract_route_info(fn_item) {
        Some(route_fn) => route_fn,
        None => return,
    };

    let module_prefix = build_module_prefix(current_module);

    let mut fixed_route_fn = route_fn;
    fixed_route_fn.module_prefix = module_prefix;

    result.push(fixed_route_fn);
}

fn handle_module(
    module: &syn::ItemMod,
    result: &mut Vec<RouteFunction>,
    current_module: &mut Vec<String>,
    path: &Path,
) {
    let module_name = module.ident.to_string();

    if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
        if file_stem == module_name {
            current_module.push(file_stem.to_string());
        }
    }

    current_module.push(module_name.clone());

    if let Some((_, ref items)) = module.content {
        for inner in items {
            process_item_with_module(inner, result, current_module, path);
        }
    }

    current_module.pop();
    if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
        if file_stem == module_name {
            current_module.pop();
        }
    }
}

#[derive(Debug)]
struct RouteFunction {
    name: String,
    method: String,
    route_path: String,
    module_prefix: String,
}

const METHOD_MAP: &[(&str, &str)] = &[
    ("get", "get"),
    ("post", "post"),
    ("put", "put"),
    ("delete", "delete"),
    ("head", "head"),
    ("connect", "connect"),
    ("options", "options"),
    ("trace", "trace"),
    ("patch", "patch"),
];

fn extract_route_info(fn_item: &ItemFn) -> Option<RouteFunction> {
    let mut method = None;
    let mut path = None;

    for attr in &fn_item.attrs {
        if is_route_attribute(attr) {
            if let Some((m, p)) = parse_route_attribute(attr) {
                method = Some(m);
                path = Some(p);
            }
        }
    }

    let name = fn_item.sig.ident.to_string();
    let method = method?;
    let route_path = path?;

    Some(RouteFunction {
        name,
        method,
        route_path,
        module_prefix: String::new(),
    })
}

fn is_route_attribute(attr: &syn::Attribute) -> bool {
    METHOD_MAP.iter().any(|&(k, _)| {
        attr.path().is_ident(k) || {
            attr.path().segments.len() == 2
                && attr.path().segments[0].ident == "actix_web"
                && attr.path().segments[1].ident == k
        }
    })
}

fn parse_route_attribute(attr: &syn::Attribute) -> Option<(String, String)> {
    let key = get_attr_key(attr)?;
    let attr_path = attr.parse_args::<LitStr>().ok()?;
    let value = attr_path.value();
    METHOD_MAP
        .iter()
        .find(|&&(k, _)| k == key)
        .map(|&(_, v)| (v.to_string(), value))
}

fn get_attr_key(attr: &syn::Attribute) -> Option<String> {
    let segments: Vec<_> = attr.path().segments.iter().collect();
    if segments.len() == 1 {
        let ident = segments[0].ident.to_string();
        return Some(ident.to_lowercase());
    }
    None
}

fn build_module_prefix(current_module: &[String]) -> String {
    if current_module.is_empty() {
        return "crate::handler".to_string();
    }

    let mut prefix =
        String::with_capacity(32 + current_module.iter().map(|s| s.len()).sum::<usize>());
    prefix.push_str("crate::handler");

    for seg in current_module {
        prefix.push_str("::");
        prefix.push_str(seg);
    }
    prefix
}
