extern crate proc_macro;

mod configure_builder;
mod tools;

use crate::configure_builder::{
    build_configure_function, generate_configure_functions_and_routes, group_functions_by_module,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use proc_macro::TokenStream;
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use syn::{parse_file, parse_macro_input, ItemFn, LitStr};

#[derive(Debug)]
struct ConfigureArgs {
    patterns: Vec<String>,
}

impl syn::parse::Parse for ConfigureArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut patterns = Vec::new();
        while !input.is_empty() {
            let path: LitStr = input.parse()?;
            patterns.push(path.value());
            if !input.is_empty() {
                let _: syn::Token![,] = input.parse()?;
            }
        }
        Ok(ConfigureArgs { patterns })
    }
}

/// generate_configure 是一个过程宏，它会扫描整个项目和 workspace 成员中的路由函数，
/// 然后自动生成 configure 函数来注册这些路由。
///
/// 它是通过 #[proc_macro] 注册的过程宏，供其他模块使用：
///
#[proc_macro]
pub fn generate_configure(input: TokenStream) -> TokenStream {
    let functions = if input.is_empty() {
        match scan_crate_for_route_functions() {
            Ok(fns) => fns,
            Err(e) => {
                return syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("Failed to scan crate for route functions: {}", e),
                )
                .to_compile_error()
                .into();
            }
        }
    } else {
        let args = parse_macro_input!(input as ConfigureArgs);
        let scan_rules = build_scan_rules(&args.patterns);
        log_scan_rules(&scan_rules);

        let files = scan_crate_for_route_files_with_rules(&scan_rules);
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
        let src_path = PathBuf::from(&manifest_dir).join("src");

        let mut result = Vec::new();
        for file in files {
            let base_module = if file.starts_with(&src_path) {
                "crate".to_string()
            } else {
                get_crate_name_from_path(&file).unwrap_or("unknown".to_string())
            };

            if let Err(e) = process_file(&file, &base_module, &mut result) {
                eprintln!("❌ Failed to process file {}: {}", file.display(), e);
            }
        }

        result
    };

    log_found_functions(&functions);

    let grouped = group_functions_by_module(&functions);
    let (all_configure_fns, all_configure_calls, all_routes) =
        generate_configure_functions_and_routes(grouped);

    let expanded = build_configure_function(all_configure_fns, all_configure_calls, all_routes);

    TokenStream::from(expanded)
}

// 构建扫描规则
fn build_scan_rules(patterns: &[String]) -> ScanRules {
    let default_exclude_patterns = vec!["!route_codegen/src/**"];
    let mut all_patterns = patterns.to_vec();
    all_patterns.extend(default_exclude_patterns.iter().cloned().map(String::from));

    let (include_patterns, exclude_patterns) = split_include_exclude(&all_patterns);
    let include_set = build_glob_set(&include_patterns).expect("Failed to build include glob set");
    let exclude_set = build_glob_set(&exclude_patterns).expect("Failed to build exclude glob set");

    ScanRules {
        include: include_set,
        exclude: exclude_set,
        include_patterns,
        exclude_patterns,
    }
}

// 打印扫描规则
fn log_scan_rules(rules: &ScanRules) {
    println!("🎯 Scan Rules:");
    println!("✅ Include patterns:");
    for pattern in &rules.include_patterns {
        println!(" - {}", pattern);
    }

    println!("❌ Exclude patterns:");
    for pattern in &rules.exclude_patterns {
        println!(" - {}", pattern);
    }
}

// 打印找到的路由函数
fn log_found_functions(functions: &[RouteFunction]) {
    println!("🔍 Found {} route functions", functions.len());
    for func in functions {
        println!(
            " - {} [{} {}] (module: {:?})",
            func.name, func.method, func.route_path, func.module_prefix
        );
    }
}

fn split_include_exclude(patterns: &[String]) -> (Vec<String>, Vec<String>) {
    let mut include = Vec::new();
    let mut exclude = Vec::new();

    for pattern in patterns {
        if pattern.starts_with('!') {
            exclude.push(pattern[1..].to_string());
        } else {
            include.push(pattern.clone());
        }
    }

    (include, exclude)
}

fn build_glob_set(patterns: &[String]) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern)?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}

#[derive(Debug)]
struct ScanRules {
    include: GlobSet,
    exclude: GlobSet,
    include_patterns: Vec<String>, // 新增字段
    exclude_patterns: Vec<String>, // 新增字段
}

impl ScanRules {
    fn should_include(&self, path: &str) -> bool {
        self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

fn scan_crate_for_route_files_with_rules(rules: &ScanRules) -> Vec<PathBuf> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let mut result = Vec::new();

    // 主项目使用 "crate" 作为根路径
    scan_project_files_with_rules(&manifest_dir, rules, &mut result, &manifest_dir);

    if let Some(workspace_config) = read_workspace_config(&manifest_dir) {
        if let Some(members) = workspace_config.members {
            let workspace_dir = PathBuf::from(&manifest_dir);
            for member in members {
                let member_dir = workspace_dir.join(&member);
                if member_dir.exists() {
                    scan_project_files_with_rules(
                        &member_dir.to_str().unwrap(),
                        rules,
                        &mut result,
                        &manifest_dir,
                    );
                }
            }
        }
    }

    result
}

fn scan_project_files_with_rules(
    manifest_dir: &str,
    rules: &ScanRules,
    result: &mut Vec<PathBuf>,
    main_dir: &str,
) {
    let src_path = PathBuf::from(manifest_dir).join("src");

    let main_or_lib_path = match find_main_or_lib(&src_path) {
        Some(p) => p,
        None => return,
    };
    println!("📦 Scanning manifest_dir: {:?}", manifest_dir);
    let root_dir = main_or_lib_path.parent().unwrap_or(&src_path);
    scan_directory_files_with_rules(root_dir, rules, result, main_dir)
}

fn scan_directory_files_with_rules<P: AsRef<Path>>(
    path: P,
    rules: &ScanRules,
    result: &mut Vec<PathBuf>,
    manifest_dir: &str,
) {
    let path = path.as_ref();

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let entry_path = entry.path();
        if should_skip_file(&entry_path, manifest_dir, rules) {
            continue;
        }
        println!("🔍 有效扫描路径 Scanning {:?}", entry_path);
        if entry_path.is_dir() {
            scan_directory_files_with_rules(&entry_path, rules, result, manifest_dir);
        } else {
            result.push(entry_path);
        }
    }
}

/// 判断是否跳过该文件
fn should_skip_file(file_path: &Path, manifest_dir: &str, rules: &ScanRules) -> bool {
    if !file_path.is_file() {
        return false;
    }

    let ext = file_path.extension().and_then(|s| s.to_str());
    if ext != Some("rs") {
        return true;
    }

    let file_name = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file_name == "main.rs" {
        return true;
    }

    let rel_path = file_path.strip_prefix(manifest_dir).unwrap_or(&file_path);

    !rules.should_include(&*normalize_path(&rel_path))
}

/// 扫描当前 crate 中所有的路由函数
fn scan_crate_for_route_functions() -> Result<Vec<RouteFunction>, String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| "CARGO_MANIFEST_DIR environment variable not found".to_string())?;

    let mut result = Vec::new();

    // 扫描主项目，使用 "crate" 作为根
    scan_project(&manifest_dir, "crate", &mut result)?;

    // 扫描工作空间成员
    if let Some(workspace_config) = read_workspace_config(&manifest_dir) {
        if let Some(members) = workspace_config.members {
            let workspace_dir = PathBuf::from(&manifest_dir);
            scan_workspace_members(workspace_dir, members, &mut result)?;
        }
    }

    Ok(result)
}

/// 遍历 workspace 成员并扫描每个成员项目的源码
fn scan_workspace_members(
    workspace_dir: PathBuf,
    members: Vec<String>,
    result: &mut Vec<RouteFunction>,
) -> Result<(), String> {
    for member in members {
        let member_dir = workspace_dir.join(&member);
        if !member_dir.exists() {
            continue;
        }

        let member_manifest_path = member_dir.join("Cargo.toml");
        if !member_manifest_path.exists() {
            continue;
        }

        // 读取成员项目的包名
        if let Some(package_name) = read_package_name(&member_manifest_path) {
            let member_manifest_dir = member_dir.to_str().unwrap().to_string();
            scan_project(&member_manifest_dir, &package_name, result)?;
        }
    }
    Ok(())
}

// 新增函数：读取 Cargo.toml 中的包名
fn read_package_name(manifest_path: &Path) -> Option<String> {
    use toml::Value;

    let mut file = fs::File::open(manifest_path).ok()?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).ok()?;

    let cargo_toml: HashMap<String, Value> = toml::from_str(&contents).ok()?;
    let package = cargo_toml.get("package")?;
    let name = package.get("name")?.as_str()?;
    Some(name.to_string())
}

/// 扫描指定项目的 src/ 目录下的所有路由函数
fn scan_project(
    manifest_dir: &str,
    crate_root: &str,
    result: &mut Vec<RouteFunction>,
) -> Result<(), String> {
    let src_path = PathBuf::from(manifest_dir).join("src");

    let main_or_lib_path = match find_main_or_lib(&src_path) {
        Some(path) => path,
        None => return Ok(()),
    };

    // 主文件所在目录
    let root_dir = main_or_lib_path.parent().unwrap_or(&src_path);

    // 计算基础模块路径
    let base_module_path = if crate_root == "crate" {
        let relative_path = root_dir.strip_prefix(&src_path).unwrap_or(root_dir);
        build_module_path("crate", relative_path)
    } else {
        let relative_path = root_dir.strip_prefix(&src_path).unwrap_or(root_dir);
        build_module_path(crate_root, relative_path)
    };

    scan_directory(root_dir, &[], &base_module_path, result)?;
    Ok(())
}

/// 构建模块路径字符串
fn build_module_path(base: &str, relative_path: &Path) -> String {
    let mut result = base.to_string();
    for comp in relative_path.components() {
        if let std::path::Component::Normal(name) = comp {
            result.push_str("::");
            result.push_str(name.to_str().unwrap());
        }
    }
    println!("📦 Scanning module: {:?}", result);
    result
}

// 读取 Cargo.toml 中的 workspace 配置
#[derive(Debug)]
struct WorkspaceConfig {
    members: Option<Vec<String>>,
}

/// 读取并解析当前项目的 Cargo.toml，提取其中的 workspace 配置
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
        let mut members_vec = Vec::new();
        for member in members {
            if let Some(member_str) = member.as_str() {
                members_vec.push(member_str.to_string());
            }
        }
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

/// 查找项目入口文件 main.rs 或 lib.rs
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

/// 递归扫描指定目录中的 .rs 源文件
fn scan_directory<P: AsRef<Path>>(
    path: P,
    exclude_files: &[&str],
    base_module_path: &str,
    result: &mut Vec<RouteFunction>,
) -> Result<(), String> {
    let path = path.as_ref();

    #[cfg(debug_assertions)]
    println!("📁 Scanning directory: {:?}", path);

    let entries = match fs::read_dir(path) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect::<Vec<_>>(),
        Err(_) => return Ok(()),
    };

    let local_results: Vec<RouteFunction> = entries
        .into_par_iter()
        .filter_map(|entry| {
            let entry_path = entry.path();
            let file_name = entry_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");

            if entry_path.is_file() {
                handle_file(&entry_path, file_name, exclude_files, base_module_path)
            } else if entry_path.is_dir() {
                handle_directory(&entry_path, base_module_path, exclude_files)
            } else {
                None
            }
        })
        .flatten()
        .collect();

    result.extend(local_results);
    Ok(())
}

/// 处理单个文件项
fn handle_file(
    entry_path: &Path,
    file_name: &str,
    exclude_files: &[&str],
    base_module_path: &str,
) -> Option<Vec<RouteFunction>> {
    let ext = entry_path.extension().and_then(|s| s.to_str());
    if ext != Some("rs") || exclude_files.contains(&file_name) {
        return None;
    }

    println!("🔍 Processing file: {:?}", entry_path);
    println!("📦 Base module path: {}", base_module_path);

    let mut sub_result = Vec::new();
    process_file(entry_path, base_module_path, &mut sub_result).ok()?;
    Some(sub_result)
}

/// 处理单个目录项
fn handle_directory(
    entry_path: &Path,
    base_module_path: &str,
    exclude_files: &[&str],
) -> Option<Vec<RouteFunction>> {
    let mut sub_result = Vec::new();
    scan_directory(entry_path, exclude_files, base_module_path, &mut sub_result).ok()?;
    Some(sub_result)
}

/// 处理单个 .rs 文件，提取其中的路由函数信息
fn process_file(
    path: &Path,
    base_module_path: &str,
    result: &mut Vec<RouteFunction>,
) -> Result<(), String> {
    // 限制最大文件大小为10MB
    const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
    let metadata = fs::metadata(path).map_err(|e| format!("Failed to get file metadata: {}", e))?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(format!("File size exceeds limit: {}", path.display()));
    }

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("❌ Failed to read file: {}", path.display());
            return Err(format!("Failed to read file: {}", e));
        }
    };

    // 解析 AST 和当前模块路径
    let mut current_module = build_current_module(base_module_path, path);

    for item in parse_file(&content)
        .map_err(|e| format!("Failed to parse file content: {}", e))?
        .items
    {
        process_item_with_module(&item, result, &mut current_module, path);
    }
    Ok(())
}

/// 构建当前文件对应的模块路径
fn build_current_module(base_module_path: &str, path: &Path) -> Vec<String> {
    let src_root = find_src_directory(path).expect("Could not find 'src' directory");
    let relative_path = path.strip_prefix(src_root).unwrap_or(path);

    let mut current_module: Vec<String> = base_module_path
        .split("::")
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    // 添加目录部分作为模块路径
    for component in relative_path.parent().unwrap_or(relative_path).components() {
        if let std::path::Component::Normal(name) = component {
            let name_str = name.to_str().unwrap();
            if name_str != "main" {
                current_module.push(name_str.to_string());
            }
        }
    }

    // 添加文件名作为模块名（排除 main.rs / lib.rs）
    if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
        if file_stem != "main" && file_stem != "lib" {
            current_module.push(file_stem.to_string());
        }
    }

    current_module
}

/// 查找包含 src 的根目录
fn find_src_directory(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("src"))
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

/// 处理函数项
fn handle_function(
    fn_item: &ItemFn,
    result: &mut Vec<RouteFunction>,
    current_module: &mut Vec<String>,
) {
    let route_fn = match extract_route_info(fn_item) {
        Some(route_fn) => route_fn,
        None => return,
    };

    // 构建模块前缀
    let module_prefix = build_module_prefix(current_module);

    let mut fixed_route_fn = route_fn;
    fixed_route_fn.module_prefix = module_prefix.to_string();

    result.push(fixed_route_fn);
}

/// 处理模块项
fn handle_module(
    module: &syn::ItemMod,
    result: &mut Vec<RouteFunction>,
    current_module: &mut Vec<String>,
    path: &Path,
) {
    let module_name = module.ident.to_string();

    // 获取当前文件名（如 agency.rs）
    let current_file_stem = path.file_stem().and_then(|s| s.to_str());

    // 如果是 agency.rs，并且模块名也是 agency，则我们手动添加两层
    if let Some(file_stem) = current_file_stem {
        if file_stem == module_name {
            // 文件名和模块名一致时，先推入文件名（模拟 crate::handler::agency）
            current_module.push(file_stem.to_string());
        }
    }

    // 再推入模块名（支持嵌套，例如 crate::handler::agency）
    current_module.push(module_name.clone());

    println!("📁 路由模块 '{}', stack: {:?}", module_name, current_module);

    // 处理模块内的项
    if let Some((_, ref items)) = module.content {
        for inner in items {
            process_item_with_module(inner, result, current_module, path);
        }
    }

    // Pop 模块名
    current_module.pop();

    // 如果是 agency.rs 的顶层模块，再 pop 掉文件名
    if let Some(file_stem) = current_file_stem {
        if file_stem == module_name {
            current_module.pop(); // 弹出文件名
        }
    }
}

/// 表示一个发现的路由函数的信息
#[derive(Clone)]
struct RouteFunction {
    name: String,          // 函数名称
    method: String,        // HTTP 方法（如 get、post）
    route_path: String,    // 路由路径（如 /api/test）
    module_prefix: String, // 新增字段：模块生成的路由前缀
}

/// 支持的 HTTP 方法列表
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

/// 提取函数上的方法属性（如 #[get(...)]）
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
        module_prefix: String::new(), // 初始化新增字段
    })
}

/// 判断属性是否是 actix-web 支持的 HTTP 方法属性（如 #[get(...)]）
fn is_route_attribute(attr: &syn::Attribute) -> bool {
    // 支持简写形式 #[get(...)] 和全路径形式 #[actix_web::get(...)]
    METHOD_MAP.iter().any(|&(k, _)| {
        attr.path().is_ident(k) || {
            attr.path().segments.len() == 2
                && attr.path().segments[0].ident == "actix_web"
                && attr.path().segments[1].ident == k
        }
    })
}

/// 解析路由属性宏的方法和路径
fn parse_route_attribute(attr: &syn::Attribute) -> Option<(String, String)> {
    let key = get_attr_key(attr)?;
    METHOD_MAP
        .iter()
        .find(|&&(k, _)| k == key)
        .and_then(|&(_, v)| {
            attr.parse_args::<LitStr>()
                .map(|attr_path| (v.to_string(), attr_path.value()))
                .ok()
        })
}

/// 提取属性宏的标识符名称
fn get_attr_key(attr: &syn::Attribute) -> Option<String> {
    let segments: Vec<_> = attr.path().segments.iter().collect();
    if segments.len() == 1 {
        let ident = segments[0].ident.to_string();
        return Some(ident.to_lowercase());
    }
    None
}

/// 构建模块前缀字符串
fn build_module_prefix(current_module: &[String]) -> Cow<'_, str> {
    let mut result = String::new();
    let mut first = true;

    for s in current_module {
        match s.as_str() {
            "crate" | "mod" => continue,
            _ => {
                if !first {
                    result.push_str("::");
                }
                result.push_str(s);
                first = false;
            }
        }
    }

    Cow::Owned(result)
}
/// 将路径标准化为 Unix 风格（使用 '/' 分隔符）
fn normalize_path<P: AsRef<Path>>(path: &P) -> Cow<'_, str> {
    let path_str = path.as_ref().to_str().unwrap_or_default();
    if path_str.contains('\\') {
        Cow::Owned(path_str.replace("\\", "/"))
    } else {
        Cow::Borrowed(path_str)
    }
}

fn get_crate_name_from_path(path: &Path) -> Option<String> {
    // 限制最大向上查找层级为10
    const MAX_PARENT_LEVELS: usize = 10;

    let mut current = path.canonicalize().ok()?;
    let mut levels = 0;

    loop {
        if levels > MAX_PARENT_LEVELS {
            break;
        }

        if current.join("Cargo.toml").exists() {
            let manifest_path = current.join("Cargo.toml");
            return read_package_name(&manifest_path);
        }

        let parent = current.parent()?;
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
        levels += 1;
    }
    None
}
