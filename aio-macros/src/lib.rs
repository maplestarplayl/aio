extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);

    let sig = &input_fn.sig;
    let block = &input_fn.block;
    let vis = &input_fn.vis;

    if sig.asyncness.is_none() {
        let msg = "the `main` function must be async";
        return syn::Error::new_spanned(sig.fn_token, msg)
            .to_compile_error()
            .into();
    }

    let result = quote! {
        #vis fn main() -> std::io::Result<()> {
            aio::spawn(async move {
                // We wrap the original async block and await it.
                // The return value of the original main is ignored.
                let _ = async #block.await;
            });
            aio::run()
        }
    };

    result.into()
}

#[proc_macro_attribute]
pub fn test(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);

    let sig = &input_fn.sig;
    let block = &input_fn.block;
    let vis = &input_fn.vis;
    let fn_name = &sig.ident;

    if sig.asyncness.is_none() {
        let msg = "the `#[aio::test]` attribute can only be used on async functions";
        return syn::Error::new_spanned(sig.fn_token, msg)
            .to_compile_error()
            .into();
    }

    let result = quote! {
        #[test]
        #vis fn #fn_name() -> std::io::Result<()> {
            // Spawn the async test block onto the runtime.
            aio::spawn(async #block);
            // Run the proactor to execute the test.
            aio::run()
        }
    };

    result.into()
}
