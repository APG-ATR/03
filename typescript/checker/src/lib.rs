#![deny(unused_must_use)]
#![deny(unreachable_patterns)]
#![deny(mutable_borrow_reservation_conflict)]
#![deny(irrefutable_let_patterns)]
#![feature(box_patterns)]
#![feature(box_syntax)]
#![feature(specialization)]
#![feature(try_blocks)]
#![feature(vec_remove_item)]
#![recursion_limit = "1024"]

#[macro_use]
extern crate swc_common;

pub use self::builtin_types::Lib;
use self::{errors::Error, legacy::Info, resolver::Resolver};
use chashmap::CHashMap;
use std::{path::PathBuf, sync::Arc};
use swc_common::{errors::Handler, Globals, SourceMap};
use swc_ecma_ast::Module;
use swc_ecma_parser::{
    lexer::Lexer, JscTarget, Parser, Session, SourceFileInput, Syntax, TsConfig,
};

pub mod analyzer;
mod builtin_types;
pub mod errors;
pub mod legacy;
pub mod loader;
pub mod resolver;
pub mod ty;
mod util;

/// Module with information.
pub type ModuleInfo = Arc<(Module, Info)>;

/// Note: All methods named `validate_*` return [Err] iff it's not recoverable.
pub type ValidationResult = ValidationResult;

#[derive(Debug)]
pub struct Config {
    /// Should we generate .d.ts?
    declaration: bool,
    /// Directory to store .d.ts files.
    declaration_dir: PathBuf,

    pub rule: Rule,
    pub libs: Vec<Lib>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Exports<T> {
    pub vars: T,
    pub types: T,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Rule {
    pub no_implicit_any: bool,
    pub no_implicit_this: bool,
    pub always_strict: bool,
    pub strict_null_checks: bool,
    pub strict_function_types: bool,

    pub allow_unreachable_code: bool,
    pub allow_unused_labels: bool,
    pub no_fallthrough_cases_in_switch: bool,
    pub no_implicit_returns: bool,
    pub suppress_excess_property_errors: bool,
    pub suppress_implicit_any_index_errors: bool,
    pub no_strict_generic_checks: bool,
    pub no_unused_locals: bool,
    pub no_unused_parameters: bool,
}

pub struct Checker<'a> {
    globals: swc_common::Globals,
    cm: Arc<SourceMap>,
    handler: &'a Handler,
    ts_config: TsConfig,
    target: JscTarget,
    /// Cache
    modules: Arc<CHashMap<PathBuf, ModuleInfo>>,
    resolver: Resolver,
    current: Arc<CHashMap<PathBuf, ()>>,
    libs: Vec<Lib>,
    rule: Rule,
}

impl<'a> Checker<'a> {
    pub fn new(
        cm: Arc<SourceMap>,
        handler: &'a Handler,
        libs: Vec<Lib>,
        rule: Rule,
        parser_config: TsConfig,
        target: JscTarget,
    ) -> Self {
        Checker {
            globals: Globals::new(),
            cm,
            handler,
            modules: Default::default(),
            ts_config: parser_config,
            target,
            resolver: Resolver::new(),
            current: Default::default(),
            libs,
            rule,
        }
    }

    pub fn run<F, R>(&self, op: F) -> R
    where
        F: FnOnce() -> R,
    {
        ::swc_common::GLOBALS.set(&self.globals, || op())
    }

    pub const fn globals(&self) -> &swc_common::Globals {
        &self.globals
    }
}

impl Checker<'_> {
    /// Returns empty vector if no error is found.
    pub fn check(&self, entry: PathBuf) -> Vec<Error> {
        self.run(|| {
            let mut errors = vec![];

            let module = self.load_module(entry.clone());

            errors.extend_from_slice(&module.1.errors);

            // let (tasks, receiver) = channel::unbounded();
            // let (result_sender, result_receiver) = channel::unbounded();
            // for import in &module.1.imports {
            //     let _ = tasks.send(Task::Resolve {
            //         from: entry.clone(),
            //         src: import.src.clone(),
            //     });
            // }

            // for i in 1..6 {
            //     let worker = Worker {
            //         sender: result_sender.clone(),
            //         queue: receiver.clone(),
            //         modules: self.modules.clone(),
            //     };
            //     thread::scope(|s| {
            //         s.spawn(|_| worker.run());
            //     })
            //     .unwrap();
            // }

            errors
        })
    }

    fn load_module(&self, path: PathBuf) -> ModuleInfo {
        let cached = self.modules.get(&path);

        if let Some(cached) = cached {
            return cached.clone();
        }

        self.current.insert(path.clone(), ());

        let module = swc_common::GLOBALS.set(&self.globals, || {
            let session = Session {
                handler: &self.handler,
            };

            let fm = self.cm.load_file(&path).expect("failed to read file");

            let lexer = Lexer::new(
                session,
                Syntax::Typescript(self.ts_config),
                self.target,
                SourceFileInput::from(&*fm),
                None,
            );
            let mut parser = Parser::new_from(session, lexer);

            parser
                .parse_typescript_module()
                .map_err(|mut e| {
                    e.emit();
                    ()
                })
                .ok()
                .unwrap_or_else(|| {
                    println!("Parser.parse_module returned Err()");
                    Module {
                        span: Default::default(),
                        body: Default::default(),
                        shebang: None,
                    }
                })
        });
        let info = self.analyze_module(Arc::new(path.clone()), &module);
        let res = Arc::new((module, info));
        self.modules.insert(path.clone(), res.clone());
        self.current.remove(&path);

        res
    }
}
