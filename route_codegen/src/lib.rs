extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
// 用于解析 Rust 源码为 AST
use rayon::prelude::*;
// 用于生成 Rust 代码的宏
use std::collections::HashMap;
// 解析 Cargo.toml 使用
use std::fs;
// 文件系统操作
use std::io::Read;
// 文件读取
use std::path::{Path, PathBuf};
// 路径处理
use syn::LitStr;
// 用于解析属性中的字符串字面量
use syn::{parse_file, ItemFn};
// 并行迭代支持

/// generate_configure 是一个过程宏，它会扫描整个项目和 workspace 成员中的路由函数，
/// 然后自动生成 configure 函数来注册这些路由。
///
/// 它是通过 #[proc_macro] 注册的过程宏，供其他模块使用：
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

    use std::collections::HashMap;

    // 按模块路径分组
    let mut grouped: HashMap<Vec<String>, Vec<RouteFunction>> = HashMap::new();
    for func in functions {
        // 将 module_prefix 拆分为模块层级列表
        let module_segments: Vec<String> = func
            .module_prefix
            .split("::")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();

        grouped
            .entry(module_segments)
            .or_insert_with(Vec::new)
            .push(func);
    }

    // 为每个模块生成 configure_xxx 函数
    let mut all_configure_fns = Vec::new();
    let mut all_configure_calls: Vec<syn::Ident> = Vec::new();
    // 新增：收集所有路由信息用于一次性打印
    let mut all_routes = Vec::new();

    for (module_path, funcs) in grouped {
        let safe_mod_name = module_path.join("_");
        let configure_ident = syn::Ident::new(
            &format!("configure_{}", safe_mod_name),
            proc_macro2::Span::call_site(),
        );

        // 自定义映射函数：过滤掉不需要出现在 URL 中的模块名

        let scope_name = module_path
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("/");

        let mod_scope = if scope_name.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", scope_name)
        };

        let services = funcs.iter().map(|f| {
            let ident = syn::Ident::new(&f.name, proc_macro2::Span::call_site());

            // 构造模块路径，并处理关键字
            let mut segments = syn::punctuated::Punctuated::new();
            for s in f.module_prefix.split("::") {
                // 使用 parse_str 构造合法的 Ident
                let ident_segment = if is_rust_keyword(s) {
                    syn::parse_str::<syn::Ident>(&format!("r#{}", s))
                        .expect("Failed to parse raw identifier")
                } else {
                    syn::parse_str::<syn::Ident>(s).expect("Failed to parse identifier")
                };
                let path_segment = syn::PathSegment::from(ident_segment);
                segments.push(path_segment);
            }

            let path = syn::Path {
                leading_colon: None,
                segments,
            };

            quote! {
                cfg.service(#path::#ident);
            }
        });

        // 收集路由信息
        for f in &funcs {
            let full_path = format!("{}{}", mod_scope, f.route_path);
            all_routes.push((f.method.to_uppercase(), full_path));
        }

        let register_ident = syn::Ident::new(
            &format!("register_{}", safe_mod_name),
            proc_macro2::Span::call_site(),
        );

        // 👇 将日志语句插入到 register 函数体中
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

    // 创建路由日志的迭代器
    let route_logs = all_routes.iter().map(|(method, path)| {
        quote! {
            log::info!("🚀 Registered route: {} {}", #method, #path);
        }
    });

    // 构建总入口 configure 函数
    let expanded = quote! {
        #(#all_configure_fns)*

        pub fn configure(cfg: &mut actix_web::web::ServiceConfig) {
            // 一次性打印所有路由信息
            {
                use std::sync::atomic::{AtomicBool, Ordering};
                static INITIALIZED: AtomicBool = AtomicBool::new(false);

                // 确保只打印一次
                if INITIALIZED.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    #(#route_logs)*
                }
            }

            #(
                cfg.configure(#all_configure_calls);
            )*
        }
    };
    // 打印最终生成的代码字符串（用于调试）
    #[cfg(debug_assertions)]
    {
        let generated_code = expanded.to_string();
        println!("🧾 Generated code:\n{}", generated_code);
    }

    TokenStream::from(expanded)
}

/// 扫描当前 crate 中所有的路由函数
fn scan_crate_for_route_functions() -> Vec<RouteFunction> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR environment variable not found");

    let mut result = Vec::new();

    // 扫描主项目，使用 "crate" 作为根
    scan_project(&manifest_dir, "crate", &mut result);

    // 扫描工作空间成员
    if let Some(workspace_config) = read_workspace_config(&manifest_dir) {
        if let Some(members) = workspace_config.members {
            let workspace_dir = PathBuf::from(&manifest_dir);
            scan_workspace_members(workspace_dir, members, &mut result);
        }
    }

    result
}

/// 遍历 workspace 成员并扫描每个成员项目的源码
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

        // 读取成员项目的包名
        if let Some(package_name) = read_package_name(&member_manifest_path) {
            let member_manifest_dir = member_dir.to_str().unwrap().to_string();
            scan_project(&member_manifest_dir, &package_name, result);
        }
    }
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
fn scan_project(manifest_dir: &str, crate_root: &str, result: &mut Vec<RouteFunction>) {
    let src_path = PathBuf::from(manifest_dir).join("src");

    let main_or_lib_path = match find_main_or_lib(&src_path) {
        Some(path) => path,
        None => return,
    };

    // 主文件所在目录
    let root_dir = main_or_lib_path.parent().unwrap_or(&src_path);

    match find_main_or_lib(&src_path) {
        Some(path) => path,
        None => return,
    };

    // 计算基础模块路径
    let base_module_path = if crate_root == "crate" {
        let relative_path = root_dir.strip_prefix(&src_path).unwrap_or(root_dir);
        let mut base = "crate".to_string();
        for comp in relative_path.components() {
            if let std::path::Component::Normal(name) = comp {
                base.push_str("::");
                base.push_str(name.to_str().unwrap());
            }
        }
        base
    } else {
        let relative_path = root_dir.strip_prefix(&src_path).unwrap_or(root_dir);
        let mut base = crate_root.to_string(); // ✅ 正确写法
        for comp in relative_path.components() {
            if let std::path::Component::Normal(name) = comp {
                base.push_str("::");
                base.push_str(name.to_str().unwrap());
            }
        }
        base
    };

    scan_directory(root_dir, &[], &base_module_path, result);
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
) {
    let path = path.as_ref();
    #[cfg(debug_assertions)]
    println!("📁 Scanning directory: {:?}", path);

    if let Ok(entries) = fs::read_dir(path) {
        let entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();

        let local_results: Vec<_> = entries
            .into_par_iter()
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
                        // 修复1：添加 base_module_path 参数
                        process_file(&entry_path, base_module_path, &mut sub_result);
                        return Some(sub_result);
                    }
                } else if entry_path.is_dir() {
                    let mut sub_result = Vec::new();
                    // 修复2：添加 base_module_path 参数
                    scan_directory(
                        &entry_path,
                        exclude_files,
                        base_module_path, // 传递 base_module_path
                        &mut sub_result,
                    );
                    return Some(sub_result);
                }

                None
            })
            .flatten()
            .collect();

        result.extend(local_results);
    } else {
        eprintln!("❌ Failed to read directory: {:?}", path);
    }
}

/// 处理单个 .rs 文件，提取其中的路由函数信息
fn process_file(path: &Path, base_module_path: &str, result: &mut Vec<RouteFunction>) {
    if let Ok(content) = fs::read_to_string(path) {
        let mut current_module: Vec<String> = base_module_path
            .split("::")
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        // 获取文件所在目录的相对路径（相对于 src）
        let src_root = path
            .ancestors()
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("src"))
            .expect("Could not find 'src' directory");

        let relative_path = path.strip_prefix(src_root).unwrap_or(path);

        // 遍历路径组件，跳过文件名，保留目录部分
        for component in relative_path.parent().unwrap_or(relative_path).components() {
            if let std::path::Component::Normal(name) = component {
                let name_str = name.to_str().unwrap();
                if name_str != "main" {
                    current_module.push(name_str.to_string());
                }
            }
        }

        // 添加当前文件名作为模块名（排除 main.rs / lib.rs / mod.rs）
        if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
            // 如果是 lib.rs 或 main.rs，则不再添加文件名为模块名
            if file_stem != "main" && file_stem != "lib" {
                current_module.push(file_stem.to_string());
            }
        }

        for item in parse_file(&content)
            .expect("Failed to parse file content")
            .items
        {
            process_item_with_module(&item, result, &mut current_module, path);
        }
    } else {
        eprintln!("❌ Failed to read file: {}", path.display());
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
    fixed_route_fn.module_prefix = module_prefix;

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

    println!(
        "📁 Entering module '{}', stack: {:?}",
        module_name, current_module
    );

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

    println!(
        "🚪 Leaving module '{}', stack now: {:?}",
        module_name, current_module
    );
}

/// 表示一个发现的路由函数的信息
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
    let attr_path = attr.parse_args::<LitStr>().ok()?;
    let value = attr_path.value();
    METHOD_MAP
        .iter()
        .find(|&&(k, _)| k == key)
        .map(|&(_, v)| (v.to_string(), value))
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
fn build_module_prefix(current_module: &[String]) -> String {
    let filtered: Vec<&str> = current_module
        .iter()
        .filter(|s| !matches!(s.as_str(), "crate" | "mod"))
        .map(String::as_str)
        .collect();
    filtered.join("::")
}

fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        "as" | "break"
            | "const"
            | "continue"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    )
}
