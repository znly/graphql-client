use failure::*;
use graphql_client_codegen::{
    generate_module_token_stream, CodegenMode, GraphQLClientCodegenOptions, TargetLang,
};
use std::{fs::File, io::Write as _, path::PathBuf};
use syn::Token;

pub(crate) struct CliCodegenParams {
    pub query_path: PathBuf,
    pub schema_path: PathBuf,
    pub selected_operation: Option<String>,
    pub additional_derives: Option<String>,
    pub deprecation_strategy: Option<String>,
    pub no_formatting: bool,
    pub module_visibility: Option<String>,
    pub output_directory: Option<PathBuf>,
    pub target_lang: Option<TargetLang>,
}

pub(crate) fn generate_code(params: CliCodegenParams) -> Result<(), failure::Error> {
    let CliCodegenParams {
        additional_derives,
        deprecation_strategy,
        no_formatting,
        output_directory,
        module_visibility: _module_visibility,
        query_path,
        schema_path,
        selected_operation,
        target_lang,
    } = params;

    let deprecation_strategy = deprecation_strategy.as_ref().and_then(|s| s.parse().ok());

    let options = {
        let mut options = GraphQLClientCodegenOptions::new(
            CodegenMode::Cli,
            target_lang.clone().unwrap_or(TargetLang::Rust),
        );
        options.set_module_visibility(
            syn::VisPublic {
                pub_token: <Token![pub]>::default(),
            }
            .into(),
        );
        if let Some(selected_operation) = selected_operation {
            options.set_operation_name(selected_operation);
        }
        if let Some(additional_derives) = additional_derives {
            options.set_additional_derives(additional_derives);
        }
        if let Some(deprecation_strategy) = deprecation_strategy {
            options.set_deprecation_strategy(deprecation_strategy);
        }
        options
    };

    match &target_lang {
        Some(TargetLang::Rust) | None => {
            let lang_options = rust::Options {
                format: !no_formatting,
                output_dir: output_directory,
            };
            rust::generate(query_path.clone(), &schema_path, options, lang_options)
        }
        Some(TargetLang::Go) => {
            let lang_options = go::Options {
                format: !no_formatting,
                output_dir: output_directory,
            };
            go::generate(query_path.clone(), &schema_path, options, lang_options)
        }
    }
}

mod rust {
    use failure::*;
    use graphql_client_codegen::{
        generate_module_token_stream, CodegenMode, GraphQLClientCodegenOptions, TargetLang,
    };
    use std::{fs::File, io::Write as _, path::PathBuf};
    use syn::Token;

    pub(super) struct Options {
        pub format: bool,
        pub output_dir: Option<PathBuf>,
    }

    pub(super) fn generate(
        query_path: std::path::PathBuf,
        schema_path: &std::path::Path,
        options: GraphQLClientCodegenOptions,
        lang_options: Options,
    ) -> Result<(), failure::Error> {
        let generated_code = generate_module_token_stream(
            &TargetLang::Rust,
            query_path.clone(),
            &schema_path,
            options,
        )?
        .to_string();
        let generated_code = if cfg!(feature = "rustfmt") && lang_options.format {
            format(&generated_code)
        } else {
            generated_code
        };

        let query_file_name: ::std::ffi::OsString = query_path
            .file_name()
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
            format_err!("Failed to find a file name in the provided query path.")
        })?;

        let dest_file_path: PathBuf = lang_options
            .output_dir
            .map(|output_dir| output_dir.join(query_file_name).with_extension("rs"))
            .unwrap_or_else(move || query_path.with_extension("rs"));

        let mut file = File::create(dest_file_path)?;
        write!(file, "{}", generated_code)?;

        Ok(())
    }

    #[allow(unused_variables)]
    fn format(codes: &str) -> String {
        #[cfg(feature = "rustfmt")]
        {
            use rustfmt::{Config, Input, Session};

            let mut config = Config::default();

            config.set().emit_mode(rustfmt_nightly::EmitMode::Stdout);
            config.set().verbose(rustfmt_nightly::Verbosity::Quiet);

            let mut out = Vec::with_capacity(codes.len() * 2);

            Session::new(config, Some(&mut out))
                .format(Input::Text(codes.to_string()))
                .unwrap_or_else(|err| panic!("rustfmt error: {}", err));

            return String::from_utf8(out).unwrap();
        }
        #[cfg(not(feature = "rustfmt"))]
        unreachable!()
    }
}

mod go {
    use failure::*;
    use graphql_client_codegen::{
        generate_module_token_stream, CodegenMode, GraphQLClientCodegenOptions, TargetLang,
    };
    use std::{fs::File, io::Write as _, path::PathBuf};
    use syn::Token;

    pub(super) struct Options {
        pub format: bool,
        pub output_dir: Option<PathBuf>,
    }

    pub(super) fn generate(
        query_path: std::path::PathBuf,
        schema_path: &std::path::Path,
        options: GraphQLClientCodegenOptions,
        lang_options: Options,
    ) -> Result<(), failure::Error> {
        let generated_code = generate_module_token_stream(
            &TargetLang::Go,
            query_path.clone(),
            &schema_path,
            options,
        )?;

        use regex::Regex;
        let re = Regex::new(r#"__JSON_TAGS_WITHOUT_OMIT \( (?P<fieldname>\w+) \)"#).unwrap();
        let contents = re
            .replace_all(&generated_code.to_string(), r##"`json:"$fieldname"`"##)
            .to_string();

        let re = Regex::new(r#"__JSON_TAGS \( (?P<fieldname>\w+) \)"#).unwrap();
        let contents = re
            .replace_all(&contents, r##"`json:"$fieldname,omitempty"`"##)
            .to_string();

        for s in contents.split("package").skip(1) {
            let file = s.split(';').next().unwrap().trim().to_string();
            let dest_file_path: PathBuf = lang_options
                .output_dir
                .as_ref()
                .map(|output_dir| output_dir.join(&file).join("query").with_extension("go"))
                .unwrap_or_else(|| query_path.join(&file).join("query").with_extension("go"));
            std::fs::create_dir_all(dest_file_path.parent().unwrap())?;
            let mut file = File::create(dest_file_path)?;
            write!(file, "package {}", s)?;
        }

        Ok(())
    }
}
