use proc_macro2::TokenStream;
use quote::quote;

pub fn write_only(block: syn::ExprBlock) -> TokenStream {
    quote! {
        #block
    }
}
