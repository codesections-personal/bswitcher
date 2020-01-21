use clap::{crate_authors, crate_name, crate_version, App, Arg};
use itertools::Itertools;
use run_script::run_script;
use serde_json::Value;
use std::str::FromStr;
use strum_macros::{Display, EnumString, EnumVariantNames};

#[derive(EnumString, Display, EnumVariantNames)]
#[strum(serialize_all = "kebab_case")]
enum SortOrder {
    FocusHistory,
    FocusHistoryCurrentFirst,
    Creation,
    Alphabetical,
}

fn main() {
    let cli = App::new(crate_name!())
        .version(crate_version!())
        .about("Interactively select a bspwm node (using dmenu) and focus that node (using bspc).")
        .arg(Arg::with_name("FORMAT_STRING").short('f').long("format-string").default_value("$line_number - $xtitle")
             .help( "The format string to use when printing the title.  The format string will be \
expanded with normal shell expansions.  The format string has access to three special variables: \
`$line_number` (the current line number of the title in dmenu, based on the SORT_ORDER), `$xtitle` \
(the raw xtitle), and `$number_of_nodes` (the total number of nodes listed in the dmenu)."))
        .arg(Arg::with_name("SORT_ORDER").short('s').long("sort-order")
             .possible_values(SortOrder::variants()).default_value("focus-history")
             .help("The order windows are displayed in dmenu.  \"focus-history\" means that the \
most-recently focused window will be displayed at the top of the list, followed by less-recently \
focused windows; the list ends with the currently focused window.  \"focus-history-current-first\" \
is the same, except the currently focused window is first, rather than last. \"creation\" means that \
windows will be listed in the order they were first created, most recent on top.  \"alphabetical\" \
lists the windows alphabetically by their xtitle (not by their formatted title)'"))
        .arg(Arg::with_name("reverse").short('r').long("reverse")
             .help("Reverses the order provided by SORT_ORDER.  Note that reversing the display \
order does not change the `$line_number` available in the FORMAT_STRING.  If you wish to calculate \
the line number after reversal, you can do so with $(($number_of_nodes - line_number))"))
        .arg(Arg::with_name("DMENU_ARGS").short('d').long("dmenu-args").default_value("-p 'Switch to: ' -l 30 -b -i")
             .help("Arguments to pass to dmenu in place of the default arguments; see dmenu(1) for \
the effects of these arguments"))        
        .arg(Arg::with_name("PIPE").short('p').long("pipe").takes_value(true)
             .help("Uses the provided pipe to modify the formatted title."))
        .after_help(&*format!(r#"EXAMPLES:
    Use defaults:
        $ bswitcher
    
    Recreate `dswitcher` menu:
        $ bswitcher -f='$((line_number + 1)) - $xtitle' -d='-p "$(date)" -l 30 -b -i' -s creation
    
    Display "Firefox" before tab title (instead of after, as in the xtitle):
        $ bswitcher --format-string '$xtitle' --pipe 'sed -E "s_(.*) - Mozilla (Firefox)_\2 | \1_"'

BUGS:
    Please report bugs to {}
"#, crate_authors!()))
    .get_matches();

    for program in &["xtitle", "dmenu", "bspc", "echo"] {
        let (code, _, _) = run_script!(format!("which {}", program)).expect("which cmd");
        if code != 0 {
            eprintln!(
                "{program} not found in $PATH.  Please install {program} before using bswitcher.",
                program = program
            );
            std::process::exit(1);
        }
    }

    let sort_order = SortOrder::from_str(cli.value_of("SORT_ORDER").unwrap()).unwrap();
    let format_string = cli.value_of("FORMAT_STRING").expect("default");
    let dmenu_args = cli.value_of("DMENU_ARGS").expect("default");
    let nodes: Vec<String> = get_nodes_from_bspwm_history();

    use SortOrder::*;
    let pipe = cli.value_of("PIPE");
    let (_, xtitles, _) = run_script!(format!("xtitle {}", nodes.join(" "))).unwrap();
    let number_of_nodes = xtitles.lines().count();
    let mut title_node_pairs: Vec<(String, String)> = xtitles
        .lines()
        .zip(nodes.iter())
        .filter(|(title, _node_id)| !title.is_empty())
        .sorted_by(
            |(a_title, a_node_id), (b_title, b_node_id)| match &sort_order {
                Alphabetical => a_title.to_lowercase().cmp(&b_title.to_lowercase()),
                Creation => a_node_id.cmp(b_node_id),
                FocusHistory | FocusHistoryCurrentFirst => std::cmp::Ordering::Equal,
            },
        )
        .enumerate()
        .map(|numbered_pair| format_xtitle(numbered_pair, format_string, number_of_nodes, pipe))
        .collect();
    if let FocusHistory = sort_order {
        let first = title_node_pairs.remove(0);
        title_node_pairs.push(first);
    };
    if cli.is_present("reverse") {
        title_node_pairs.reverse();
    }

    let titles = title_node_pairs
        .iter()
        .fold(String::new(), |list, (title, _)| list + &title);

    let (_, out, _) =
        run_script!(format!(r#"echo -n '{}' | dmenu {}"#, titles, dmenu_args)).unwrap();

    let target_node = title_node_pairs
        .iter()
        .find(|(title, _)| &out == title)
        .map(|(_, node)| node)
        .unwrap_or_else(|| {
            std::process::exit(1);
        });
    run_script!(format!("bspc node --focus {}", target_node)).unwrap();
}

fn get_nodes_from_bspwm_history() -> Vec<String> {
    let state: Value = serde_json::from_str(&run_script!(r"bspc wm -d").unwrap().1).unwrap();
    let hist: Vec<Value> = serde_json::from_str(&format!("{}", state["focusHistory"])).unwrap();
    hist.iter()
        .map(|hist_item| hist_item["nodeId"].to_string())
        .rev()
        .unique()
        .collect()
}

fn format_xtitle(
    (i, (xtitle, node_id)): (usize, (&str, &String)),
    format_string: &str,
    number_of_nodes: usize,
    pipe: Option<&str>,
) -> (String, String) {
    let line_number = i.to_string();
    let xtitle = xtitle.replace("'", "’"); // avoid ending the bash string
    let mut format_cmd = format!(
        r#"
line_number='{}'
xtitle='{}'
number_of_nodes='{}'
echo "{}""#,
        line_number, xtitle, number_of_nodes, format_string
    );
    if let Some(pipe) = pipe {
        format_cmd.push_str(&format!("| {}", pipe));
    }
    let (code, title, err) = run_script!(format_cmd).unwrap();
    if code != 0 {
        eprintln!("format_xtitle: {}", err);
        std::process::exit(2);
    }
    (title, node_id.to_string())
}
