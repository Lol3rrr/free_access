use proc_macro::TokenStream;
use syn::{parse_macro_input, AttributeArgs};

mod wrapper;
mod write_only;

#[proc_macro_attribute]
pub fn freeaccess(attr: TokenStream, input: TokenStream) -> TokenStream {
    let input_impl: syn::ItemFn = parse_macro_input!(input);
    let attributes = parse_macro_input!(attr as AttributeArgs);

    wrapper::wrapper(attributes, input_impl).into()
}

#[proc_macro]
pub fn write_only(input: TokenStream) -> TokenStream {
    let input_scope: syn::ExprBlock = parse_macro_input!(input);

    write_only::write_only(input_scope).into()
}
