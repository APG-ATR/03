use super::{
    export::ExportExtra,
    expr::{any, never_ty},
    scope::{ScopeKind, VarInfo},
    util::TypeRefExt,
    Analyzer,
};
use crate::errors::Error;
use fxhash::FxHashMap;
use std::{borrow::Cow, ops::AddAssign};
use swc_atoms::JsWord;
use swc_common::{Spanned, Visit, VisitWith};
use swc_ecma_ast::*;

#[derive(Debug, Default)]
struct Facts {
    true_facts: CondFacts,
    false_facts: CondFacts,
}

impl AddAssign for Facts {
    fn add_assign(&mut self, rhs: Self) {
        self.true_facts += rhs.true_facts;
        self.false_facts += rhs.false_facts;
    }
}

impl AddAssign<Option<Self>> for Facts {
    fn add_assign(&mut self, rhs: Option<Self>) {
        match rhs {
            Some(rhs) => {
                *self += rhs;
            }
            None => {}
        }
    }
}

/// Conditional facts
#[derive(Debug, Default)]
struct CondFacts {
    types: FxHashMap<JsWord, VarInfo>,
}

impl AddAssign for CondFacts {
    fn add_assign(&mut self, rhs: Self) {
        self.types.extend(rhs.types);
    }
}

impl AddAssign<Option<Self>> for CondFacts {
    fn add_assign(&mut self, rhs: Option<Self>) {
        match rhs {
            Some(rhs) => {
                *self += rhs;
            }
            None => {}
        }
    }
}

impl Analyzer<'_, '_> {
    pub(super) fn try_assign(&mut self, lhs: &PatOrExpr, ty: Cow<TsType>) {
        match *lhs {
            PatOrExpr::Expr(ref expr) | PatOrExpr::Pat(box Pat::Expr(ref expr)) => match **expr {
                // TODO(kdy1): Validate
                Expr::Member(MemberExpr { .. }) => return,
                _ => unimplemented!(
                    "assign: {:?} = {:?}\nFile: {}",
                    expr,
                    ty,
                    self.path.display()
                ),
            },

            PatOrExpr::Pat(ref pat) => {
                // Update variable's type
                match **pat {
                    Pat::Ident(ref i) => {
                        if let Some(var_info) = self.scope.vars.get_mut(&i.sym) {
                            // Variable is declared.

                            let var_ty = if let Some(ref var_ty) = var_info.ty {
                                // let foo: string;
                                // let foo = 'value';

                                let errors = ty.assign_to(&var_ty);
                                if errors.is_none() {
                                    Some(ty.into_owned())
                                } else {
                                    self.info.errors.extend(errors);
                                    None
                                }
                            } else {
                                // let v = foo;
                                // v = bar;
                                None
                            };
                            if let Some(var_ty) = var_ty {
                                if var_info.ty.is_none() || !var_info.ty.as_ref().unwrap().is_any()
                                {
                                    var_info.ty = Some(var_ty);
                                }
                            }
                        } else {
                            let var_info = if let Some(var_info) = self.scope.search_parent(&i.sym)
                            {
                                VarInfo {
                                    ty: if var_info.ty.is_some()
                                        && var_info.ty.as_ref().unwrap().is_any()
                                    {
                                        Some(any(var_info.ty.as_ref().unwrap().span()))
                                    } else {
                                        Some(ty.into_owned())
                                    },
                                    copied: true,
                                    ..var_info.clone()
                                }
                            } else {
                                if let Some(extra) = self
                                    .scope
                                    .find_type(&i.sym)
                                    .as_ref()
                                    .and_then(|v| v.extra.as_ref())
                                {
                                    match extra {
                                        ExportExtra::Module(..) => {
                                            self.info.errors.push(Error::NotVariable {
                                                span: i.span,
                                                left: lhs.span(),
                                            });

                                            return;
                                        }
                                        _ => {}
                                    }
                                }
                                // undefined symbol
                                self.info
                                    .errors
                                    .push(Error::UndefinedSymbol { span: i.span });
                                return;
                            };
                            // Variable is defined on parent scope.
                            //
                            // We copy varinfo with enhanced type.
                            self.scope.vars.insert(i.sym.clone(), var_info);
                        }
                    }

                    _ => unimplemented!("assignment with complex pattern"),
                }
            }
        }
    }

    fn add_true_false(&self, facts: &mut Facts, sym: &JsWord, ty: Cow<TsType>) {
        macro_rules! base {
            () => {{
                match self.find_var(sym) {
                    Some(v) => VarInfo {
                        copied: true,
                        ..v.clone()
                    },
                    None => {
                        unimplemented!("error reporting: add_true_false: undefined symbol {}", sym)
                    }
                }
            }};
        }

        facts.true_facts.types.insert(
            sym.clone(),
            VarInfo {
                ty: Some(ty.clone().into_owned().remove_falsy()),
                ..base!()
            },
        );
        facts.false_facts.types.insert(
            sym.clone(),
            VarInfo {
                ty: Some(ty.into_owned().remove_truthy()),
                ..base!()
            },
        );
    }

    fn visit_flow<F>(&mut self, op: F)
    where
        F: for<'a, 'b> FnOnce(&mut Analyzer<'a, 'b>),
    {
        let errors = {
            let mut child = self.child(ScopeKind::Flow);

            op(&mut child);

            assert_eq!(
                child.info.exports,
                Default::default(),
                "Child node cannot export"
            );
            child.info.errors
        };

        self.info.errors.extend(errors);
    }

    /// Returns (type facts when test is matched, type facts when test is not
    /// matched)
    fn detect_facts(&self, test: &Expr, facts: &mut Facts) -> Result<(), Error> {
        match *test {
            // Useless
            Expr::Fn(..)
            | Expr::Arrow(..)
            | Expr::Lit(Lit::Bool(..))
            | Expr::Lit(Lit::Str(..))
            | Expr::Lit(Lit::Null(..))
            | Expr::Lit(Lit::Num(..))
            | Expr::MetaProp(..)
            | Expr::JSXFragment(..)
            | Expr::JSXNamespacedName(..)
            | Expr::JSXEmpty(..) => return Ok(()),

            // Object literal *may* have side effect.
            Expr::Object(..) => {}

            // Array literal *may* have side effect.
            Expr::Array(..) => {}

            Expr::Await(AwaitExpr { arg: ref expr, .. })
            | Expr::TsNonNull(TsNonNullExpr { ref expr, .. }) => {
                self.detect_facts(expr, facts)?;
            }

            Expr::Seq(SeqExpr { ref exprs, .. }) => {
                for expr in exprs {
                    self.detect_facts(expr, facts)?;
                }
            }

            Expr::Paren(ParenExpr { ref expr, .. }) => self.detect_facts(expr, facts)?,

            Expr::Ident(ref i) => {
                let ty = self.type_of(test)?;
                self.add_true_false(facts, &i.sym, ty);
            }

            Expr::Bin(BinExpr {
                op: op!("&&"),
                ref left,
                ref right,
                ..
            }) => {
                self.detect_facts(left, facts)?;
                self.detect_facts(right, facts)?;
            }

            Expr::Bin(BinExpr {
                op: op!("==="),
                ref left,
                ref right,
                ..
            }) => {
                let l_ty = self.type_of(left)?;
                let r_ty = self.type_of(right)?;
            }

            _ => unimplemented!("detect_facts({:?})", test),
        }

        Ok(())
    }
}

impl Visit<IfStmt> for Analyzer<'_, '_> {
    fn visit(&mut self, stmt: &IfStmt) {
        let mut facts = Default::default();
        let facts = self.detect_facts(&stmt.test, &mut facts);
        self.visit_flow(|child| {
            if stmt.cons.ends_with_ret() {

            } else {

            }

            //
            stmt.visit_children(child)
        });
    }
}

pub(super) trait RemoveTypes {
    /// Removes falsy values from `self`.
    fn remove_falsy(self) -> TsType;

    /// Removes truthy values from `self`.
    fn remove_truthy(self) -> TsType;
}

impl RemoveTypes for TsType {
    fn remove_falsy(self) -> TsType {
        match self {
            TsType::TsUnionOrIntersectionType(n) => n.remove_falsy().into(),
            TsType::TsKeywordType(TsKeywordType { kind, span }) => match kind {
                TsKeywordTypeKind::TsUndefinedKeyword | TsKeywordTypeKind::TsNullKeyword => {
                    never_ty(span)
                }
                _ => self,
            },
            TsType::TsLitType(ty) => match ty.lit {
                TsLit::Bool(Bool { value: false, span }) => never_ty(span),
                _ => TsType::TsLitType(ty),
            },
            _ => self,
        }
    }

    fn remove_truthy(self) -> TsType {
        match self {
            TsType::TsUnionOrIntersectionType(n) => n.remove_truthy().into(),
            TsType::TsLitType(ty) => match ty.lit {
                TsLit::Bool(Bool { value: true, span }) => never_ty(span),
                _ => TsType::TsLitType(ty),
            },
            _ => self,
        }
    }
}

impl RemoveTypes for TsUnionOrIntersectionType {
    fn remove_falsy(self) -> TsType {
        match self {
            TsUnionOrIntersectionType::TsIntersectionType(n) => n.remove_falsy().into(),
            TsUnionOrIntersectionType::TsUnionType(n) => n.remove_falsy().into(),
        }
    }

    fn remove_truthy(self) -> TsType {
        match self {
            TsUnionOrIntersectionType::TsIntersectionType(n) => n.remove_truthy().into(),
            TsUnionOrIntersectionType::TsUnionType(n) => n.remove_truthy().into(),
        }
    }
}

impl RemoveTypes for TsIntersectionType {
    fn remove_falsy(self) -> TsType {
        let types = self
            .types
            .into_iter()
            .map(|ty| ty.remove_falsy())
            .map(Box::new)
            .collect::<Vec<_>>();
        if types.iter().any(|ty| is_never(&ty)) {
            return TsType::TsKeywordType(TsKeywordType {
                span: self.span,
                kind: TsKeywordTypeKind::TsNeverKeyword,
            });
        }

        TsType::TsUnionOrIntersectionType(TsIntersectionType { types, ..self }.into())
    }

    fn remove_truthy(self) -> TsType {
        let types = self
            .types
            .into_iter()
            .map(|ty| ty.remove_truthy())
            .map(Box::new)
            .collect::<Vec<_>>();
        if types.iter().any(|ty| is_never(&ty)) {
            return TsType::TsKeywordType(TsKeywordType {
                span: self.span,
                kind: TsKeywordTypeKind::TsNeverKeyword,
            });
        }

        TsType::TsUnionOrIntersectionType(TsIntersectionType { types, ..self }.into())
    }
}

impl RemoveTypes for TsUnionType {
    fn remove_falsy(mut self) -> TsType {
        let types = self
            .types
            .into_iter()
            .map(|ty| box ty.remove_falsy())
            .filter(|ty| !is_never(ty))
            .collect();
        TsType::TsUnionOrIntersectionType(TsUnionType { types, ..self }.into())
    }

    fn remove_truthy(mut self) -> TsType {
        let types = self
            .types
            .into_iter()
            .map(|ty| box ty.remove_truthy())
            .filter(|ty| !is_never(ty))
            .collect();
        TsType::TsUnionOrIntersectionType(TsUnionType { types, ..self }.into())
    }
}

impl RemoveTypes for Box<TsType> {
    fn remove_falsy(self) -> TsType {
        (*self).remove_falsy()
    }

    fn remove_truthy(self) -> TsType {
        (*self).remove_truthy()
    }
}

trait EndsWithRet {
    /// Returns true if the statement ends with return, break, continue;
    fn ends_with_ret(&self) -> bool;
}

impl EndsWithRet for Stmt {
    /// Returns true if the statement ends with return, break, continue;
    fn ends_with_ret(&self) -> bool {
        match *self {
            Stmt::Return(..) | Stmt::Break(..) | Stmt::Continue(..) => true,
            _ => false,
        }
    }
}

impl EndsWithRet for BlockStmt {
    /// Returns true if the statement ends with return, break, continue;
    fn ends_with_ret(&self) -> bool {
        self.stmts.ends_with_ret()
    }
}

impl<T> EndsWithRet for Vec<T>
where
    T: EndsWithRet,
{
    /// Returns true if the statement ends with return, break, continue;
    fn ends_with_ret(&self) -> bool {
        match self.last() {
            Some(ref stmt) => stmt.ends_with_ret(),
            _ => false,
        }
    }
}

fn is_never(ty: &TsType) -> bool {
    match *ty {
        TsType::TsKeywordType(TsKeywordType {
            kind: TsKeywordTypeKind::TsNeverKeyword,
            ..
        }) => false,
        _ => true,
    }
}