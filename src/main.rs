//!

use std::collections::HashSet;
use std::env;
use std::fmt::{self, Display};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

use proc_macro2::{TokenStream, TokenTree};
use syn::{
    Expr, FnArg, ForeignItem, Item, ItemConst, ItemFn, ItemForeignMod, ItemMacro, ItemStruct,
    ItemType, ItemUse, Lit, Pat, PathArguments, ReturnType, Type, TypePath, UseTree, Visibility,
};
#[allow(unused)]
enum Error {
    IncorrectUsage,
    ReadFile(io::Error),
    ParseFile {
        error: syn::Error,
        filepath: PathBuf,
        source_code: String,
    },
    Unhandled(String),
    Nyi,
}

struct Cx {
    link_name: String,
    toplevel_imports: HashSet<String>,
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Error::*;

        match self {
            IncorrectUsage => write!(f, "Usage: dump-syntax path/to/filename.rs"),
            ReadFile(error) => write!(f, "Unable to read file: {}", error),
            ParseFile { error, .. } => write!(f, "Unable to parse file: {}", error),
            Unhandled(item_name) => write!(f, "Unhandled item {}", item_name),
            Nyi => write!(f, "Not yet implemented"),
        }
    }
}

fn main() {
    if let Err(error) = try_main() {
        let _ = writeln!(io::stderr(), "{}", error);
        process::exit(1);
    }
}

fn path_as_single_ident(path: &syn::Path) -> Option<String> {
    if path.segments.len() == 1 {
        let seg = &path.segments[0];
        if seg.arguments == PathArguments::None {
            return Some(seg.ident.to_string());
        }
    }
    None
}

fn ty_to_zig(ty: &Type) -> Result<String, Error> {
    match ty {
        Type::Path(TypePath { path, .. }) => {
            if path.segments.len() == 1 {
                let seg = &path.segments[0];
                if seg.arguments == PathArguments::None {
                    let mut ident = seg.ident.to_string();
                    // Zig doesn't have c char types.
                    match ident.as_str() {
                        "c_uchar" => ident = "u8".into(),
                        "c_char" | "c_schar" => ident = "i8".into(),
                        "__uint64" => ident = "u64".into(),
                        "__int64" => ident = "i64".into(),
                        _ => (),
                    }
                    return Ok(ident);
                }
            }
        }
        Type::Ptr(p) => {
            let mut_str = if p.const_token.is_some() {
                "const "
            } else {
                ""
            };
            return Ok(format!("?*{}{}", mut_str, ty_to_zig(&p.elem)?));
        }
        _ => (),
    }
    Err(Error::Nyi)
}

fn ret_ty_to_zig(r: &ReturnType) -> Result<String, Error> {
    match r {
        ReturnType::Type(_, t) => ty_to_zig(t),
        ReturnType::Default => Ok("".to_string()),
    }
}

fn vis_to_zig(v: &Visibility) -> &str {
    if matches!(v, Visibility::Public(_)) {
        "pub "
    } else {
        ""
    }
}

fn expr_to_zig(e: &Expr) -> String {
    match e {
        Expr::Lit(l) => match &l.lit {
            Lit::Int(i) => return i.to_string(),
            _ => (),
        },
        _ => (),
    }
    "???".into()
}

type UsePath = Vec<String>;

/// Expand a use tree into individual paths.
fn expand_use_tree(u: &UseTree) -> Result<Vec<UsePath>, Error> {
    fn expand_rec(u: &UseTree, prefix: &[String], b: &mut Vec<UsePath>) -> Result<(), Error> {
        match u {
            UseTree::Path(p) => {
                let mut path = prefix.to_owned();
                path.push(p.ident.to_string());
                expand_rec(&p.tree, &path, b)?;
            }
            UseTree::Name(n) => {
                let mut path = prefix.to_owned();
                path.push(n.ident.to_string());
                b.push(path);
            }
            UseTree::Group(g) => {
                for tree in &g.items {
                    expand_rec(tree, prefix, b)?;
                }
            }
            _ => return Err(Error::Nyi),
        }
        Ok(())
    }
    let mut b = Vec::new();
    expand_rec(u, &[], &mut b)?;
    Ok(b)
}

fn use_to_zig(u: &ItemUse, cx: &mut Cx) -> Result<(), Error> {
    for path in expand_use_tree(&u.tree)? {
        let toplevel = &path[0];
        if toplevel != "ctypes" {
            if !cx.toplevel_imports.contains(toplevel) {
                println!("");
                println!("const {} = @import(\"{}.zig\");", toplevel, toplevel);
            }
            cx.toplevel_imports.insert(toplevel.clone());
            let last = path.last().unwrap();
            let vis = vis_to_zig(&u.vis);
            let import = path.join(".");
            println!("{}const {} = {};", vis, last, import);
        }
    }
    Ok(())
}

fn const_to_zig(c: &ItemConst) {
    //println!("{:#?}", c);
    let vis = vis_to_zig(&c.vis);
    println!("{}const {} = {};", vis, c.ident, expr_to_zig(&c.expr));
}

fn type_to_zig(t: &ItemType) -> Result<(), Error> {
    //println!("{:#?}", t);
    let vis = vis_to_zig(&t.vis);
    let ident = t.ident.to_string();
    println!("{}const {} = {};", vis, ident, ty_to_zig(&t.ty)?);
    Ok(())
}

fn fn_arg_to_zig(arg: &FnArg) -> Result<(), Error> {
    //println!("{:?}", arg);
    let mut ident = String::new();
    if let FnArg::Typed(t) = arg {
        match t.pat.as_ref() {
            Pat::Ident(i) => ident = i.ident.to_string(),
            Pat::Wild(_) => ident = "_".to_string(),
            _ => (),
        }
        println!("    {}: {},", ident, ty_to_zig(&t.ty)?);
    }
    Ok(())
}

fn foreign_mod_to_zig(fm: &ItemForeignMod, cx: &Cx) -> Result<(), Error> {
    //println!("{:#?}", fm);
    for item in &fm.items {
        match item {
            ForeignItem::Fn(f) => {
                let vis = vis_to_zig(&f.vis);
                println!("{}extern \"{}\" fn {} (", vis, cx.link_name, &f.sig.ident);
                for arg in &f.sig.inputs {
                    fn_arg_to_zig(arg)?;
                }
                println!(") callconv(.Stdcall) {};", ret_ty_to_zig(&f.sig.output)?)
            }
            _ => println!("{:?}", item),
        }
    }
    Ok(())
}

fn struct_macro_to_zig(toks: &TokenStream) -> Result<(), Error> {
    let s: ItemStruct = syn::parse2(toks.to_owned()).unwrap();
    //println!("STRUCT! {:?}", s);
    println!("pub const {} = extern struct {{", s.ident);
    for f in &s.fields {
        println!("    {}: {},", f.ident.as_ref().unwrap(), ty_to_zig(&f.ty)?);
    }
    println!("}};");
    Ok(())
}

fn declare_handle_to_zig(toks: &TokenStream) -> Result<(), Error> {
    let mut tok_iter = toks.clone().into_iter();
    let handle_id = tok_iter.next().ok_or(Error::Nyi)?;
    // Skip comma. We *should* check, but meh.
    tok_iter.next();
    let opaque_id = tok_iter.next().ok_or(Error::Nyi)?;
    if let (TokenTree::Ident(h), TokenTree::Ident(o)) = (handle_id, opaque_id) {
        println!("pub const {} = @Type(.Opaque);", o.to_string());
        println!("pub const {} = ?*{};", h.to_string(), o.to_string());
    }
    Ok(())
}

fn macro_to_zig(m: &ItemMacro) -> Result<(), Error> {
    if let Some(id) = path_as_single_ident(&m.mac.path) {
        match id.as_str() {
            "STRUCT" => struct_macro_to_zig(&m.mac.tokens),
            "DECLARE_HANDLE" => declare_handle_to_zig(&m.mac.tokens),
            _ => Err(Error::Unhandled(id)),
        }
    } else {
        Err(Error::Nyi)
    }
}

fn fn_to_zig(f: &ItemFn) -> Result<(), Error> {
    Err(Error::Unhandled(f.sig.ident.to_string()))
}

fn item_to_zig(item: &Item, cx: &mut Cx) -> Result<(), Error> {
    match item {
        Item::Use(u) => use_to_zig(u, cx)?,
        Item::Type(t) => type_to_zig(t)?,
        Item::Const(c) => const_to_zig(c),
        Item::ForeignMod(fm) => foreign_mod_to_zig(fm, cx)?,
        Item::Macro(m) => macro_to_zig(m)?,
        Item::Fn(f) => fn_to_zig(f)?,
        _ => println!("{:#?}", item),
    }
    Ok(())
}

fn wrap_item_to_zig(item: &Item, cx: &mut Cx) -> Result<(), Error> {
    let result = item_to_zig(item, cx);
    match result {
        Err(Error::Unhandled(item_name)) => {
            println!("// Unhandled item: {}", item_name);
            return Ok(());
        }
        Err(Error::Nyi) => {
            println!("// Item not yet implemented");
            return Ok(());
        }
        _ => (),
    }
    result
}

fn try_main() -> Result<(), Error> {
    let mut args = env::args_os();
    let _ = args.next(); // executable name

    let filepath = match (args.next(), args.next()) {
        (Some(arg), None) => PathBuf::from(arg),
        _ => return Err(Error::IncorrectUsage),
    };

    let code = fs::read_to_string(&filepath).map_err(Error::ReadFile)?;
    let syntax = syn::parse_file(&code).map_err({
        |error| Error::ParseFile {
            error,
            filepath,
            source_code: code,
        }
    })?;
    let mut cx = Cx {
        link_name: "user32".into(),
        toplevel_imports: Default::default(),
    };
    for item in &syntax.items {
        wrap_item_to_zig(item, &mut cx)?;
    }

    Ok(())
}
