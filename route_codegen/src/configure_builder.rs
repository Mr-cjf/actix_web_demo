use crate::tools::is_rust_keyword;
// 导入 is_rust_keyword 函数
use crate::RouteFunction;
use quote::quote;
use syn::{punctuated::Punctuated, Ident, PathSegment, Token};

/// 按模块路径分组
pub fn group_functions_by_module(
    functions: &[RouteFunction],
) -> std::collections::HashMap<Vec<String>, Vec<RouteFunction>> {
    let mut grouped: std::collections::HashMap<Vec<String>, Vec<RouteFunction>> =
        std::collections::HashMap::new();
    for func in functions {
        let module_segments: Vec<String> = func
            .module_prefix
            .split("::")
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();

        grouped
            .entry(module_segments)
            .or_insert_with(Vec::new)
            .push(func.clone());
    }
    grouped
}

/// 生成 configure_xxx 和 register_xxx 函数及路由信息
pub fn generate_configure_functions_and_routes(
    grouped: std::collections::HashMap<Vec<String>, Vec<RouteFunction>>,
) -> (
    Vec<proc_macro2::TokenStream>,
    Vec<Ident>,
    Vec<(String, String)>,
) {
    let mut all_configure_fns = Vec::new();
    let mut all_configure_calls = Vec::new();
    let mut all_routes = Vec::new();

    for (module_path, functions) in grouped {
        let (configure_fn, register_fn, calls, routes) =
            generate_module_configure(&module_path, &functions);
        all_configure_fns.push(register_fn);
        all_configure_fns.push(configure_fn);
        all_configure_calls.extend(calls);
        all_routes.extend(routes);
    }

    (all_configure_fns, all_configure_calls, all_routes)
}

/// 为每个模块生成 configure/register 函数及相关内容
fn generate_module_configure(
    module_path: &[String],
    functions: &[RouteFunction],
) -> (
    proc_macro2::TokenStream,
    proc_macro2::TokenStream,
    Vec<Ident>,
    Vec<(String, String)>,
) {
    let safe_mod_name = module_path.join("_");
    let configure_ident = Ident::new(
        &format!("configure_{}", safe_mod_name),
        proc_macro2::Span::call_site(),
    );

    let scope_name = module_path.join("/");
    let mod_scope = if scope_name.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", scope_name)
    };

    let services = functions.iter().map(|f| {
        let ident = Ident::new(&f.name, proc_macro2::Span::call_site());

        let mut segments = Punctuated::<PathSegment, Token![::]>::new();
        for s in f.module_prefix.split("::") {
            let ident_segment = if is_rust_keyword(s) {
                Ident::new(&format!("r#{}", s), proc_macro2::Span::call_site())
            } else {
                Ident::new(s, proc_macro2::Span::call_site())
            };
            segments.push(PathSegment::from(ident_segment));
        }

        quote! {
            cfg.service(#segments::#ident);
        }
    });

    let register_ident = Ident::new(
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

    let routes = functions
        .iter()
        .map(|f| {
            (
                f.method.to_uppercase(),
                format!("{}{}", mod_scope, f.route_path),
            )
        })
        .collect();

    (configure_fn, register_fn, vec![configure_ident], routes)
}

/// 构建最终的 configure 函数
pub fn build_configure_function(
    all_configure_fns: Vec<proc_macro2::TokenStream>,
    all_configure_calls: Vec<Ident>,
    all_routes: Vec<(String, String)>,
) -> proc_macro2::TokenStream {
    let route_logs = all_routes.iter().map(|(method, path)| {
        quote! {
            log::info!("🚀 Registered route: {} {}", #method, #path);
        }
    });

    let configure_all = quote! {
        #(#all_configure_fns)*

        pub fn configure(cfg: &mut actix_web::web::ServiceConfig) {
            {
                use std::sync::atomic::{AtomicBool, Ordering};
                static INITIALIZED: AtomicBool = AtomicBool::new(false);

                if INITIALIZED.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    #(#route_logs)*
                }
            }

            #(
                cfg.configure(#all_configure_calls);
            )*
        }
    };

    configure_all
}
