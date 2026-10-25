use accord::{car_read, catalog, compiler, symbols, xcode::XcodeVersion};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const BUNDLE_VERSION: &str = "24506";
const SHORT_BUNDLE_VERSION: &str = "26.3";

#[derive(Parser, Debug)]
#[command(name = "accord", about = "Compiles, prints, updates, and verifies asset catalogs.",
          args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// .xcassets and .icon documents, compiled into one catalog
    documents: Vec<PathBuf>,

    /// Output format
    #[arg(long, default_value = "xml1",
          value_parser = ["xml1", "binary1", "human-readable-text"])]
    output_format: String,

    #[arg(long)]
    compile: Option<PathBuf>,

    #[arg(long)]
    warnings: bool,

    #[arg(long)]
    errors: bool,

    #[arg(long)]
    notices: bool,

    #[arg(long)]
    output_partial_info_plist: Option<PathBuf>,

    #[arg(long)]
    app_icon: Option<String>,

    #[arg(long)]
    include_all_app_icons: bool,

    #[arg(long, action = clap::ArgAction::Append, default_value = "")]
    alternate_app_icon: Vec<String>,

    #[arg(long)]
    accent_color: Option<String>,

    #[arg(long)]
    widget_background_color: Option<String>,

    #[arg(long, default_value = "macosx")]
    platform: String,

    #[arg(long, default_value = "11.0")]
    minimum_deployment_target: String,

    #[arg(long, action = clap::ArgAction::Append, default_value = "")]
    target_device: Vec<String>,

    #[arg(long, default_value = "default",
          value_parser = ["default", "all", "none"])]
    standalone_icon_behavior: String,

    #[arg(long)]
    compress_pngs: bool,

    #[arg(long)]
    skip_app_store_deployment: bool,

    #[arg(long, default_value = "NO")]
    enable_on_demand_resources: String,

    /// `NO` leaves out the pre-rendered icons under `--xcode-version 26`; ignored under 27
    #[arg(long)]
    enable_icon_stack_fallback_generation: Option<String>,

    /// accord only: the Xcode build whose actool output to reproduce
    #[arg(long, default_value = "27", value_parser = ["26", "27"])]
    xcode_version: String,

    /// Accepted and ignored
    #[arg(long)]
    force_png: bool,

    #[arg(long)]
    bundle_identifier: Option<String>,

    #[arg(long)]
    development_region: Option<String>,

    #[arg(long, action = clap::ArgAction::Append, default_value = "")]
    include_language: Vec<String>,

    #[arg(long, default_value = "YES")]
    include_partial_info_plist_localizations: String,

    #[arg(long)]
    export_dependency_info: Option<PathBuf>,

    #[arg(long)]
    generate_objc_asset_symbols: Option<PathBuf>,

    #[arg(long)]
    generate_asset_symbol_index: Option<PathBuf>,

    #[arg(long)]
    print_contents: bool,

    #[arg(long)]
    version: bool,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Print the structure of a compiled asset catalog (.car)
    Inspect {
        car: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Structural differences between two .car files (`want` is the reference)
    Diff { want: PathBuf, got: PathBuf },
    /// Write every decodable image rendition as PNG into `out`
    Extract { car: PathBuf, out: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match &cli.command {
        Some(Command::Inspect { car, json }) => return inspect(car, *json),
        Some(Command::Diff { want, got }) => return diff(want, got),
        Some(Command::Extract { car, out }) => return extract(car, out),
        None => {}
    }

    if cli.version {
        print_version(&cli.output_format);
        return ExitCode::SUCCESS;
    }

    let Some(document) = cli.documents.first().cloned() else {
        eprintln!("error: a document path is required");
        return ExitCode::from(2);
    };

    if cli.print_contents && cli.compile.is_none() {
        match catalog::AssetCatalog::new(
            document.clone(),
            cli.platform.clone(),
            cli.minimum_deployment_target.clone(),
            None,
            None,
            None,
        )
        .parse()
        {
            Ok(_) => {
                print_plist_xml("com.apple.actool.catalog-contents", "(empty)");
            }
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        }
        return ExitCode::SUCCESS;
    }

    let Some(compile_dir) = cli.compile.clone() else {
        eprintln!("error: --compile is required");
        return ExitCode::from(2);
    };

    // --generate-objc-asset-symbols with --bundle-identifier replaces normal
    // compilation: only the header (and optional index) is produced.
    if let (Some(header_path), Some(bundle_id)) =
        (cli.generate_objc_asset_symbols.as_ref(), cli.bundle_identifier.as_ref())
    {
        if let Err(e) = symbols::generate_symbols_header(
            &document,
            header_path,
            bundle_id,
            &cli.platform,
        ) {
            eprintln!("error generating header: {e}");
            return ExitCode::from(1);
        }
        let mut output_files: Vec<PathBuf> = Vec::new();
        if let Some(index_path) = cli.generate_asset_symbol_index.as_ref() {
            if let Err(e) =
                symbols::generate_symbol_index(&document, index_path, &cli.platform)
            {
                eprintln!("error generating index: {e}");
                return ExitCode::from(1);
            }
            output_files.push(
                std::fs::canonicalize(index_path).unwrap_or_else(|_| index_path.clone()),
            );
        }
        output_files.push(
            std::fs::canonicalize(header_path).unwrap_or_else(|_| header_path.clone()),
        );
        if let Some(dep) = cli.export_dependency_info.as_ref() {
            if let Err(e) = write_dependency_info(dep, std::slice::from_ref(&document), &output_files) {
                eprintln!("error writing deps: {e}");
                return ExitCode::from(1);
            }
        }
        print_compilation_results(&cli.output_format, &output_files);
        return ExitCode::SUCCESS;
    }

    let include_languages: Option<Vec<String>> = {
        let v: Vec<String> =
            cli.include_language.iter().filter(|s| !s.is_empty()).cloned().collect();
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    };
    let plist_l10n = !matches!(
        cli.include_partial_info_plist_localizations.to_ascii_uppercase().as_str(),
        "NO"
    );

    let opts = compiler::Options {
        platform: cli.platform.clone(),
        min_deploy: cli.minimum_deployment_target.clone(),
        app_icon: cli.app_icon.clone(),
        info_plist: cli.output_partial_info_plist.clone(),
        accent_color: cli.accent_color.clone(),
        widget_background_color: cli.widget_background_color.clone(),
        standalone_icon_behavior: cli.standalone_icon_behavior.clone(),
        target_devices: cli.target_device.iter().filter(|d| !d.is_empty()).cloned().collect(),
        include_all_app_icons: cli.include_all_app_icons,
        include_languages,
        development_region: cli.development_region.clone(),
        plist_localizations: plist_l10n,
        xcode_version: XcodeVersion::parse(&cli.xcode_version).expect("clap allows only known versions"),
        disable_icon_stack_fallback: cli
            .enable_icon_stack_fallback_generation
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("NO")),
    };
    let output_files = match compiler::compile(&cli.documents, &compile_dir, &opts) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(1);
        }
    };

    if let Some(dep) = cli.export_dependency_info.as_ref() {
        if let Err(e) = write_dependency_info(dep, &cli.documents, &output_files) {
            eprintln!("error writing deps: {e}");
            return ExitCode::from(1);
        }
    }

    print_compilation_results(&cli.output_format, &output_files);
    ExitCode::SUCCESS
}

fn print_version(format: &str) {
    let mut s = String::new();
    s.push_str(&format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>com.apple.actool.version</key>
	<dict>
		<key>bundle-version</key>
		<string>{BUNDLE_VERSION}</string>
		<key>short-bundle-version</key>
		<string>{SHORT_BUNDLE_VERSION}</string>
	</dict>
</dict>
</plist>
"#
    ));
    if format == "human-readable-text" {
        println!("/* com.apple.actool.version */");
        println!("bundle-version: {BUNDLE_VERSION}");
        println!("short-bundle-version: {SHORT_BUNDLE_VERSION}");
    } else {
        print!("{s}");
    }
}

fn print_plist_xml(_top_key: &str, _body: &str) {
    // TODO: print the catalog-contents plist.
}

fn print_compilation_results(format: &str, output_files: &[PathBuf]) {
    if format == "human-readable-text" {
        println!("/* com.apple.actool.compilation-results */");
        for f in output_files {
            println!("{}", f.display());
        }
        return;
    }
    let mut s = String::new();
    s.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    s.push('\n');
    s.push_str(r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#);
    s.push('\n');
    s.push_str(r#"<plist version="1.0">"#);
    s.push('\n');
    s.push_str("<dict>\n");
    s.push_str("\t<key>com.apple.actool.compilation-results</key>\n");
    s.push_str("\t<dict>\n");
    s.push_str("\t\t<key>output-files</key>\n");
    s.push_str("\t\t<array>\n");
    for f in output_files {
        s.push_str(&format!("\t\t\t<string>{}</string>\n", f.display()));
    }
    s.push_str("\t\t</array>\n");
    s.push_str("\t</dict>\n");
    s.push_str("</dict>\n");
    s.push_str("</plist>\n");
    print!("{s}");
}

fn write_dependency_info(
    path: &Path,
    inputs: &[PathBuf],
    output_files: &[PathBuf],
) -> std::io::Result<()> {
    let mut out: Vec<u8> = Vec::new();
    out.push(0x00);
    out.extend_from_slice(format!("actool-{BUNDLE_VERSION}").as_bytes());
    out.push(0x00);
    for input in inputs {
        let abs_input = std::fs::canonicalize(input).unwrap_or_else(|_| input.clone());
        out.push(0x10);
        out.extend_from_slice(abs_input.to_string_lossy().as_bytes());
        out.push(0x00);
    }
    for f in output_files {
        out.push(0x40);
        out.extend_from_slice(f.to_string_lossy().as_bytes());
        out.push(0x00);
    }
    std::fs::write(path, out)
}

fn read_car(path: &Path) -> Result<car_read::Car, ExitCode> {
    std::fs::read(path).map_err(anyhow::Error::from).and_then(|d| car_read::Car::parse(&d)).map_err(|e| {
        eprintln!("error: {}: {e:#}", path.display());
        ExitCode::from(1)
    })
}

fn diff(want: &Path, got: &Path) -> ExitCode {
    let (want, got) = match (read_car(want), read_car(got)) {
        (Ok(w), Ok(g)) => (w, g),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let lines = car_read::diff(&want, &got);
    lines.iter().for_each(|l| println!("{l}"));
    if lines.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

fn extract(path: &Path, out: &Path) -> ExitCode {
    let car = match read_car(path) {
        Ok(car) => car,
        Err(e) => return e,
    };
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("error: {}: {e}", out.display());
        return ExitCode::from(1);
    }
    let file_name = |r: &car_read::Rendition, with_identifier: bool| {
        let key: Vec<String> = car.keyformat.iter().zip(&r.key)
            .filter(|(a, v)| **v != 0 && ![1u16, 2].contains(a) && (with_identifier || **a != 17))
            .map(|(a, v)| format!("{}{v}", car_read::attribute_name(*a)))
            .collect();
        let stem = car_read::normalize_name(&r.name).replace(['/', '<', '>'], "_");
        format!("{stem}.{}.png", key.join("."))
    };
    // Identifiers differ between compilers; names keep them only where renditions would collide
    // (every `.icon` layer is "image.png").
    let mut uses = std::collections::HashMap::new();
    for r in car.renditions.iter().filter(|r| r.rgba8().is_some()) {
        *uses.entry(file_name(r, false)).or_insert(0) += 1;
    }
    for r in &car.renditions {
        let Some(rgba) = r.rgba8() else { continue };
        let name = file_name(r, false);
        let file = out.join(if uses[&name] > 1 { file_name(r, true) } else { name });
        let img = image::RgbaImage::from_raw(r.width, r.height, rgba).expect("sized by rgba8");
        if let Err(e) = img.save(&file) {
            eprintln!("error: {}: {e}", file.display());
            return ExitCode::from(1);
        }
        println!("{}", file.display());
    }
    ExitCode::SUCCESS
}

fn inspect(path: &Path, json: bool) -> ExitCode {
    let car = match read_car(path) {
        Ok(car) => car,
        Err(e) => return e,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&car).expect("serializable"));
        return ExitCode::SUCCESS;
    }
    if let Some(h) = &car.header {
        println!("CoreUI {} storage {} schema {} key-semantics {}  {} renditions",
                 h.coreui_version, h.storage_version, h.schema_version, h.key_semantics, h.rendition_count);
        println!("  {}", h.version_string.trim());
    }
    if let Some(m) = &car.metadata {
        println!("  {} {}  {}", m.deployment_platform, m.deployment_platform_version, m.authoring_tool.trim());
    }
    let attrs: Vec<String> = car.keyformat.iter().map(|&a| car_read::attribute_name(a)).collect();
    println!("keyformat: {}", attrs.join(" "));
    for (name, id) in &car.appearances {
        println!("appearance {id}: {name}");
    }
    for (name, id) in &car.localizations {
        println!("localization {id}: {name}");
    }
    for f in &car.facets {
        let a: Vec<String> = f.attributes.iter().map(|(k, v)| format!("{}={v}", car_read::attribute_name(*k))).collect();
        println!("facet {:?}: {}", f.name, a.join(" "));
    }
    for r in &car.renditions {
        let key: Vec<String> = car.keyformat.iter().zip(&r.key)
            .filter(|(_, v)| **v != 0)
            .map(|(a, v)| format!("{}={v}", car_read::attribute_name(*a)))
            .collect();
        println!("{:<20} {:>4}x{:<4} @{} {:<4} {:?}  [{}]", car_read::layout_name(r.layout), r.width, r.height,
                 r.scale, r.pixel_format, r.name, key.join(" "));
        let payload = serde_json::to_string(&r.payload).expect("serializable");
        println!("    payload {payload}");
        for t in &r.tlvs {
            println!("    tlv {}", serde_json::to_string(t).expect("serializable"));
        }
    }
    ExitCode::SUCCESS
}
