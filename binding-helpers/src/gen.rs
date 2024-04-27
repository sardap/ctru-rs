use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;

use bindgen::callbacks::{
    DeriveInfo, DeriveTrait, FieldInfo, ImplementsTrait, ParseCallbacks, TypeKind,
};
use bindgen::FieldVisibilityKind;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, TokenStreamExt};

use regex::Regex;
#[cfg(feature = "rustfmt")]
use rust_format::{Formatter, RustFmt};

#[derive(Debug)]
pub struct LayoutTestCallbacks(Rc<LayoutTestGenerator>);

impl LayoutTestCallbacks {
    pub fn new() -> (Self, Rc<LayoutTestGenerator>) {
        let generator = Rc::new(LayoutTestGenerator::new());
        (Self(Rc::clone(&generator)), generator)
    }
}

impl ParseCallbacks for LayoutTestCallbacks {
    fn header_file(&self, filename: &str) {
        self.0.headers.borrow_mut().insert(filename.to_string());
    }

    fn add_derives(&self, info: &DeriveInfo<'_>) -> Vec<String> {
        match info.kind {
            TypeKind::Struct | TypeKind::Enum => {
                self.0
                    .fields
                    .borrow_mut()
                    .insert(info.name.to_string(), HashSet::new());
            }
            TypeKind::Union => {
                // layout tests don't handle unions for now, just skip it
                println!(
                    "cargo:warning=Skipping generated tests for union {}",
                    info.name,
                );
                self.0.blocklist_type(info.name);
            }
        }

        Vec::new()
    }

    fn blocklisted_type_implements_trait(
        &self,
        name: &str,
        _derive_trait: DeriveTrait,
    ) -> Option<ImplementsTrait> {
        self.0.blocklist_type(name);
        None
    }

    fn field_visibility(&self, info: FieldInfo<'_>) -> Option<FieldVisibilityKind> {
        self.0
            .fields
            .borrow_mut()
            .entry(info.type_name.to_string())
            .or_default()
            .insert(info.field_name.to_string());

        None
    }
}

#[derive(Debug)]
pub struct LayoutTestGenerator {
    fields: RefCell<HashMap<String, HashSet<String>>>,
    blocklist: RefCell<Vec<Regex>>,
    headers: RefCell<HashSet<String>>,
}

impl LayoutTestGenerator {
    fn new() -> Self {
        Self {
            fields: Default::default(),
            blocklist: Default::default(),
            headers: Default::default(),
        }
    }

    pub fn blocklist_type(&self, pattern: &str) -> &Self {
        self.blocklist
            .borrow_mut()
            .push(Regex::new(pattern).unwrap());
        self
    }

    pub fn generate_layout_tests(&self, output_path: impl AsRef<Path>) -> Result<(), crate::Error> {
        let mut file = File::create(output_path)?;

        // Since quote! tokenizes its input, it would result in invalid C++ for
        // the `#include` directives (no whitespace/newlines), so we basically
        // need to drop in the include headers here "manually" by writing them
        // into the cpp! macro invocation.
        file.write_all(b"cpp! {{\n")?;
        for included_file in self.headers.borrow().iter() {
            writeln!(file, "    #include \"{included_file}\"")?;
        }
        file.write_all(b"}}\n\n")?;

        let test_tokens = self.build_tests();

        file.write_all(
            #[cfg(feature = "rustfmt")]
            RustFmt::default().format_tokens(test_tokens)?.as_bytes(),
            #[cfg(not(feature = "rustfmt"))]
            test_tokens.to_string().as_bytes(),
        )?;

        Ok(())
    }

    fn build_tests(&self) -> TokenStream {
        let mut output = TokenStream::new();

        output.append_all(build_preamble());

        'structs: for struct_name in self.fields.borrow().keys() {
            for pattern in self.blocklist.borrow().iter() {
                if pattern.is_match(struct_name) {
                    continue 'structs;
                }
            }
            output.append_all(self.build_struct_test(struct_name));
        }

        output
    }

    fn build_struct_test(&self, struct_name: &str) -> proc_macro2::TokenStream {
        let name = format_ident!("{struct_name}");

        let test_name = format_ident!("layout_test_{struct_name}");

        let mut field_tests = Vec::new();
        field_tests.push(build_assert_eq(
            quote!(size_of!(#name)),
            quote!(sizeof(#name)),
        ));
        field_tests.push(build_assert_eq(
            quote!(align_of!(#name)),
            quote!(alignof(#name)),
        ));

        for field in self.fields.borrow().get(struct_name).into_iter().flatten() {
            let field = format_ident!("{field}");

            field_tests.push(build_assert_eq(
                quote!(size_of!(#name::#field)),
                quote!(sizeof(#name::#field)),
            ));

            field_tests.push(build_assert_eq(
                quote!(align_of!(#name::#field)),
                quote!(alignof(#name::#field)),
            ));

            field_tests.push(build_assert_eq(
                quote!(offset_of!(#name, #field)),
                quote!(offsetof(#name, #field)),
            ));
        }

        quote! {
            #[test]
            fn #test_name() {
                #(#field_tests);*
            }
        }
    }
}

fn build_preamble() -> TokenStream {
    quote! {
        use cpp::cpp;

        macro_rules! size_of {
            ($ty:ident::$field:ident) => {{
                size_of_ret(|x: $ty| x.$field)
            }};
            ($ty:ty) => {
                ::std::mem::size_of::<$ty>()
            };
            ($expr:expr) => {
                ::std::mem::size_of_val(&$expr)
            };
        }

        macro_rules! align_of {
            ($ty:ident::$field:ident) => {{
                align_of_ret(|x: $ty| x.$field)
            }};
            ($ty:ty) => {
                ::std::mem::align_of::<$ty>()
            };
            ($expr:expr) => {
                ::std::mem::align_of_val(&$expr)
            };
        }

        fn size_of_ret<T, U>(_f: impl Fn(U) -> T) -> usize {
            ::std::mem::size_of::<T>()
        }

        fn align_of_ret<T, U>(_f: impl Fn(U) -> T) -> usize {
            ::std::mem::align_of::<T>()
        }
    }
}

fn build_assert_eq(rust_lhs: TokenStream, cpp_rhs: TokenStream) -> TokenStream {
    quote! {
        assert_eq!(
            #rust_lhs,
            cpp!(unsafe [] -> usize as "size_t" { return #cpp_rhs; }),
            "{} != {}",
            stringify!(#rust_lhs),
            stringify!(#cpp_rhs),
        );
    }
}
