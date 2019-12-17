use clap::{crate_authors, crate_name, crate_version, App, Arg};
use itertools::Itertools;
use run_script::run_script;
use serde_json::Value;
use std::collections::HashMap;
use strfmt::strfmt;

fn main() {
    let cli = App::new(crate_name!())
        .author(crate_authors!())
        .version(crate_version!())
        .arg("-f --format [FORMAT_STRING] 'The format string to use when printing the title (see note below).'")
        .arg(Arg::with_name("SORT_ORDER")
             .short('s')
             .long("--sort-order")
             .possible_values(&["focus-history",
                                "focus-history-current-first",
                                "creation",
                                "alphabetical"])
             .default_value("focus-history")
             .help("The display order for windows via dmenu"))
        .get_matches();
    let nodes: Vec<String> = get_nodes();

    let (_, xtitles, _) = run_script!(format!("xtitle {}", nodes.join(" "))).unwrap();

    let mut titles_and_nodes: Vec<(String, String)> = xtitles
        .lines()
        .zip(nodes.iter())
        .filter(|(title, _id)| !title.is_empty())
        .map(|(title, id)| (title.to_string(), id.to_string()))
        .sorted_by(|(a_title, a_node), (b_title, b_node)| {
            match cli.value_of("SORT_ORDER").unwrap() {
                "alphabetical" => a_title.to_lowercase().cmp(&b_title.to_lowercase()),
                "creation" => a_node.cmp(b_node),
                _ => a_node.cmp(a_node),
            }
        })
        .enumerate()
        .map(|(number, (title, node_id))| {
            let mut vars = HashMap::new();
            let i = (number + 1).to_string();
            vars.insert("title".to_string(), title);
            vars.insert("number".to_string(), i);
            let fmt = cli.value_of("format").unwrap_or("{number} - {title}");
            let fmt_titles = strfmt(&fmt, &vars).unwrap_or_else(|_| {
                eprintln!("Invalid FORMAT_STRING supplied.");
                std::process::exit(1);
            });
            (fmt_titles, node_id.to_string())
        })
        .collect();
    if cli.value_of("SORT_ORDER").unwrap() == "focus-history" {
        let first = titles_and_nodes.remove(0);
        titles_and_nodes.push(first);
    };

    let titles = titles_and_nodes
        .iter()
        .fold(String::new(), |list, pair| format!("{}{}\n", list, pair.0));

    let (_, mut out, _) = run_script!(format!(r#"echo -n "{}" | dmenu -l 9 -b"#, titles)).unwrap();
    out.pop(); //trim newline

    let target_node = titles_and_nodes
        .iter()
        .find(|(title, _)| &out == title)
        .map(|(_, node)| node)
        .unwrap_or_else(|| {
            std::process::exit(1);
        });

    run_script!(format!("bspc node --focus {}", target_node)).unwrap();
}

fn get_nodes() -> Vec<String> {
    let state: Value = serde_json::from_str(&run_script!(r"bspc wm -d").unwrap().1).unwrap();
    let hist: Vec<Value> = serde_json::from_str(&format!("{}", state["focusHistory"])).unwrap();
    hist.iter()
        .map(|hist_item| hist_item["nodeId"].to_string())
        .rev()
        .unique()
        .collect()
}
