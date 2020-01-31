use clap::{crate_authors, crate_name, crate_version, App, Arg, ArgMatches};
use itertools::Itertools;
use serde_json::Value;
use std::{error::Error, str::FromStr};
use strum_macros::{Display, EnumString, EnumVariantNames};
use utils::{dependencies, sh, Die};

#[derive(EnumString, Display, Clone, Copy, EnumVariantNames)]
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
        .arg(Arg::with_name("FORMAT_STRING")
             .short('f')
             .long("format-string")
             .default_value("$line_number - $xtitle")
             .help( "The format string to use when printing the title.  The format string will be \
expanded with normal shell expansions.  The format string has access to three special variables: \
`$line_number` (the current line number of the title in dmenu, based on the SORT_ORDER), `$xtitle` \
(the raw xtitle), and `$number_of_nodes` (the total number of nodes listed in the dmenu)."))
        .arg(Arg::with_name("SORT_ORDER")
             .short('s')
             .long("sort-order")
             .possible_values(SortOrder::variants())
             .default_value("focus-history")
             .help("The order windows are displayed in dmenu.  \"focus-history\" means that the \
most-recently focused window will be displayed at the top of the list, followed by less-recently \
focused windows; the list ends with the currently focused window.  \"focus-history-current-first\" \
is the same, except the currently focused window is first, rather than last. \"creation\" means that \
windows will be listed in the order they were first created, most recent on top.  \"alphabetical\" \
lists the windows alphabetically by their xtitle (not by their formatted title)'"))
        .arg(Arg::with_name("reverse")
             .short('r')
             .long("reverse")
             .help("Reverses the order provided by SORT_ORDER.  Note that reversing the display \
order does not change the `$line_number` available in the FORMAT_STRING.  If you wish to calculate \
the line number after reversal, you can do so with $(($number_of_nodes - line_number))"))
        .arg(Arg::with_name("DMENU_ARGS")
             .short('d')
             .long("dmenu-args")
             .allow_hyphen_values(true)
             .default_value("-p 'Switch to: ' -l 30 -b -i")
             .help("Arguments to pass to dmenu in place of the default arguments; see dmenu(1) for \
the effects of these arguments."))        
        .arg(Arg::with_name("PIPE")
             .short('p')
             .long("pipe")
             .takes_value(true)
             .help("Uses the provided pipe to modify the formatted title."))
        .arg(Arg::from("--src 'Prints this program's source to stdout'"))
        .after_help(&*format!(r#"EXAMPLES:
    Use defaults:
        $ bswitcher
    
    Recreate `dswitcher` menu:
        $ bswitcher -f='$((line_number + 1)) - $xtitle' -d='-p "$(date)" -l 30 -b -i' -s creation
    
    Display "Firefox" before tab title (instead of after, as in the xtitle):
        $ bswitcher --format-string '$xtitle' --pipe 'sed -E "s_(.*) - Mozilla (Firefox)_\2 | \1_"'

BUGS:
    Please report bugs to {}"#, crate_authors!()))
    .get_matches();
    run(cli).unwrap_or_die();
}

fn run(cli: ArgMatches) -> Result<(), Box<dyn Error>> {
    if cli.is_present("src") {
        print!("/// main.rs\n{}", include_str!("main.rs"));
        return Ok(());
    }
    dependencies(vec!["xtitle", "dmenu", "bspc", "echo"])?;
    let sort_order =
        SortOrder::from_str(cli.value_of("SORT_ORDER").expect("default")).expect("possible-values");
    let nodes_in_history_order = {
        let full_bspwm_state = sh("bspc wm --dump-state")?.0.parse::<Value>()?;
        let bspwm_focus_history: Vec<Value> =
            serde_json::from_str(&full_bspwm_state["focusHistory"].to_string())?;
        bspwm_focus_history
            .iter()
            .map(|hist_item| hist_item["nodeId"].to_string())
            .rev()
            .unique()
            .collect::<Vec<String>>()
    };
    let (xtitles, _) = sh(&format!("xtitle {}", nodes_in_history_order.join(" ")))?;

    let (titles, nodes): (Vec<String>, Vec<String>) = {
        // Pair titles and nodes
        let nodes = xtitles
            .lines()
            .zip(nodes_in_history_order.iter())
            .filter(|(title, _node_id)| !title.is_empty());

        // Sort them
        use SortOrder::*;
        let mut sorted_nodes = nodes
            .sorted_by(|(a_title, a_id), (b_title, b_id)| match &sort_order {
                Alphabetical => a_title.to_lowercase().cmp(&b_title.to_lowercase()),
                Creation => a_id.cmp(b_id),
                FocusHistory | FocusHistoryCurrentFirst => std::cmp::Ordering::Equal,
            })
            .collect::<Vec<(&str, &String)>>();
        if let SortOrder::FocusHistory = sort_order {
            let first = sorted_nodes.remove(0);
            sorted_nodes.push(first);
        };

        // Apply formatting
        let mut formated_nodes: Vec<(String, String)> = sorted_nodes
            .iter()
            .enumerate()
            .map(|(line_number, (xtitle, node_id))| {
                match sh(&format!(
                    r#"
line_number='{line_number}'
xtitle='{xtitle}'
number_of_nodes='{number_of_nodes}'
echo "{format_string}" {pipe}"#,
                    line_number = line_number,
                    xtitle = xtitle.replace("'", "’"), // avoid ending the bash string
                    number_of_nodes = xtitles.lines().count(),
                    format_string = cli.value_of("FORMAT_STRING").expect("default"),
                    pipe = match cli.value_of("PIPE") {
                        Some(pipe) => format!("| {}", pipe),
                        None => format!(""),
                    }
                )) {
                    Err(e) => Err(e.into()),
                    Ok((out, _err)) => Ok((out, node_id.to_string())),
                }
            })
            .collect::<Result<_, Box<dyn Error>>>()?;

        // Maybe reverse them (after formatting to keep line numbers non-reversed)
        if cli.is_present("reverse") {
            formated_nodes.reverse();
        }

        formated_nodes.iter().cloned().unzip()
    };

    let (target_title, _err) = sh(&format!(
        r#"echo -n '{titles}' | dmenu {dmenu_args}"#,
        titles = titles.join(""),
        dmenu_args = cli.value_of("DMENU_ARGS").expect("default"),
    ))?;

    sh(&format!(
        "bspc node --focus {target_node}",
        target_node = &nodes[titles
            .iter()
            .position(|title| title == &target_title)
            .expect("Found title in same vec")]
    ))?;
    Ok(())
}
