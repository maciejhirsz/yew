use crate::html_tree::HtmlDashedName;
use std::{
    cmp::Ordering,
    convert::TryFrom,
    ops::{Deref, DerefMut},
};
use syn::{
    parse::{Parse, ParseStream},
    Block, Expr, ExprBlock, Stmt, Token,
};

pub struct Prop {
    pub label: HtmlDashedName,
    /// Punctuation between `label` and `value`.
    pub value: Expr,
}
impl Parse for Prop {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let label = input.parse::<HtmlDashedName>()?;
        let equals = input.parse::<Token![=]>().map_err(|_| {
            syn::Error::new_spanned(
                &label,
                format!("`{}` doesn't have a value. (hint: set the value to `true` or `false` for boolean attributes)", label),
            )
        })?;
        if input.is_empty() {
            return Err(syn::Error::new_spanned(
                equals,
                "expected an expression following this equals sign",
            ));
        }
        let value = strip_braces(input.parse::<Expr>()?)?;
        Ok(Self { label, value })
    }
}

fn strip_braces(expr: Expr) -> syn::Result<Expr> {
    match expr {
        Expr::Block(ExprBlock { block: Block { mut stmts, .. }, .. }) if stmts.len() == 1 => {
            let stmt = stmts.remove(0);
            match stmt {
                Stmt::Expr(expr) => Ok(expr),
                Stmt::Semi(_expr, semi) => Err(syn::Error::new_spanned(
                        semi,
                        "only an expression may be assigned as a property. Consider removing this semicolon",
                )),
                _ =>             Err(syn::Error::new_spanned(
                        stmt,
                        "only an expression may be assigned as a property",
                ))
            }
        }
        Expr::Lit(_) | Expr::Block(_) => Ok(expr),
        _ => Err(syn::Error::new_spanned(
                &expr,
                "the property value must be either a literal or enclosed in braces. Consider adding braces around your expression.".to_string(),
        )),
    }
}

/// List of props sorted in alphabetical order*.
///
/// \*The "children" prop always comes last to match the behaviour of the `Properties` derive macro.
///
/// The list may contain multiple props with the same label.
/// Use `check_no_duplicates` to ensure that there are no duplicates.
pub struct SortedPropList(Vec<Prop>);
impl SortedPropList {
    const CHILDREN_LABEL: &'static str = "children";

    /// Create a new `SortedPropList` from a vector of props.
    /// The given `props` doesn't need to be sorted.
    pub fn new(mut props: Vec<Prop>) -> Self {
        props.sort_by(|a, b| Self::cmp_label(&a.label.to_string(), &b.label.to_string()));
        Self(props)
    }

    fn cmp_label(a: &str, b: &str) -> Ordering {
        if a == b {
            Ordering::Equal
        } else if a == Self::CHILDREN_LABEL {
            Ordering::Greater
        } else if b == Self::CHILDREN_LABEL {
            Ordering::Less
        } else {
            a.cmp(b)
        }
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.0
            .binary_search_by(|prop| Self::cmp_label(prop.label.to_string().as_str(), key))
            .ok()
    }

    /// Get the first prop with the given key.
    pub fn get_by_label(&self, key: &str) -> Option<&Prop> {
        self.position(key).and_then(|i| self.0.get(i))
    }

    /// Pop the first prop with the given key.
    pub fn pop(&mut self, key: &str) -> Option<Prop> {
        self.position(key).map(|i| self.0.remove(i))
    }

    /// Pop the prop with the given key and error if there are multiple ones.
    pub fn pop_unique(&mut self, key: &str) -> syn::Result<Option<Prop>> {
        let prop = self.pop(key);
        if prop.is_some() {
            if let Some(other_prop) = self.get_by_label(key) {
                return Err(syn::Error::new_spanned(
                    &other_prop.label,
                    format!("`{}` can only be specified once", key),
                ));
            }
        }

        Ok(prop)
    }

    /// Turn the props into a vector of `Prop`.
    pub fn into_vec(self) -> Vec<Prop> {
        self.0
    }

    /// Iterate over all duplicate props in order of appearance.
    fn iter_duplicates(&self) -> impl Iterator<Item = &Prop> {
        self.0.windows(2).filter_map(|pair| {
            let (a, b) = (&pair[0], &pair[1]);

            if a.label == b.label {
                Some(b)
            } else {
                None
            }
        })
    }

    /// Remove and return all props for which `filter` returns `true`.
    pub fn drain_filter(&mut self, filter: impl FnMut(&Prop) -> bool) -> Self {
        let (drained, others) = self.0.drain(..).partition(filter);
        self.0 = others;
        Self(drained)
    }

    /// Run the given function for all props and aggregate the errors.
    /// If there's at least one error, the result will be `Result::Err`.
    pub fn check_all(&self, f: impl FnMut(&Prop) -> syn::Result<()>) -> syn::Result<()> {
        crate::join_errors(self.0.iter().map(f).filter_map(Result::err))
    }

    /// Return an error for all duplicate props.
    pub fn check_no_duplicates(&self) -> syn::Result<()> {
        crate::join_errors(self.iter_duplicates().map(|prop| {
            syn::Error::new_spanned(
                &prop.label,
                format!(
                    "`{}` can only be specified once but is given here again",
                    prop.label
                ),
            )
        }))
    }
}
impl Parse for SortedPropList {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut props: Vec<Prop> = Vec::new();
        while !input.is_empty() {
            props.push(input.parse()?);
        }

        Ok(Self::new(props))
    }
}
impl Deref for SortedPropList {
    type Target = [Prop];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Default)]
pub struct SpecialProps {
    pub node_ref: Option<Prop>,
    pub key: Option<Prop>,
}
impl SpecialProps {
    const REF_LABEL: &'static str = "ref";
    const KEY_LABEL: &'static str = "key";

    fn pop_from(props: &mut SortedPropList) -> syn::Result<Self> {
        let node_ref = props.pop_unique(Self::REF_LABEL)?;
        let key = props.pop_unique(Self::KEY_LABEL)?;
        Ok(Self { node_ref, key })
    }

    pub fn get_slot_mut(&mut self, key: &str) -> Option<&mut Option<Prop>> {
        match key {
            Self::REF_LABEL => Some(&mut self.node_ref),
            Self::KEY_LABEL => Some(&mut self.key),
            _ => None,
        }
    }

    fn iter(&self) -> impl Iterator<Item = &Prop> {
        self.node_ref.as_ref().into_iter().chain(self.key.as_ref())
    }

    /// Run the given function for all props and aggregate the errors.
    /// If there's at least one error, the result will be `Result::Err`.
    pub fn check_all(&self, f: impl FnMut(&Prop) -> syn::Result<()>) -> syn::Result<()> {
        crate::join_errors(self.iter().map(f).filter_map(Result::err))
    }
}

pub struct Props {
    pub special: SpecialProps,
    pub prop_list: SortedPropList,
}
impl Parse for Props {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Self::try_from(input.parse::<SortedPropList>()?)
    }
}
impl Deref for Props {
    type Target = SortedPropList;

    fn deref(&self) -> &Self::Target {
        &self.prop_list
    }
}
impl DerefMut for Props {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.prop_list
    }
}

impl TryFrom<SortedPropList> for Props {
    type Error = syn::Error;

    fn try_from(mut prop_list: SortedPropList) -> Result<Self, Self::Error> {
        let special = SpecialProps::pop_from(&mut prop_list)?;
        Ok(Self { special, prop_list })
    }
}
