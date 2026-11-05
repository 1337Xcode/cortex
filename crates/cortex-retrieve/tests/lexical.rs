//! The prompt's words looked up in a store written by hand, with how well each file matches worked
//! out from the rows each word is in.

mod common;

use common::{Repo, TestResult, function};
use cortex_core::FileFlags;
use cortex_retrieve::{Graph, GraphSettings, LexicalMatch, PromptTerms, lexical, read_graph};
use cortex_store::NodeId;

/// A shop of three files in `shop`: `orders.py` with `create_order` (lines 1 to 5) and
/// `cancel_order` (7 to 9), `users.py` with `create_user` (1 to 4), `billing.py` with `charge`
/// (1 to 3). Its search rows, eight with the folder's: `shop`; `shop orders py`, `create order
/// create order`, `cancel order cancel order`; `shop users py`, `create user create user`; `shop
/// billing py`, `charge charge`. So `shop` is in 4 rows, `py` in 3, `create` and `order` in 2, every
/// other word in 1.
struct Shop {
    /// The repository.
    repo: Repo,
    /// `orders.py`, `create_order`, `cancel_order`.
    orders: Vec<NodeId>,
    /// `users.py`, `create_user`.
    users: Vec<NodeId>,
    /// `billing.py`, `charge`.
    billing: Vec<NodeId>,
}

impl Shop {
    /// The shop.
    fn new() -> TestResult<Self> {
        let mut repo = Repo::new()?;
        let lines = "line\n".repeat(9);
        let orders = repo.file(
            "shop/orders.py",
            &lines,
            &[
                function("create_order", 1, 5),
                function("cancel_order", 7, 9),
            ],
            FileFlags::empty(),
        )?;
        let users = repo.file(
            "shop/users.py",
            &lines,
            &[function("create_user", 1, 4)],
            FileFlags::empty(),
        )?;
        let billing = repo.file(
            "shop/billing.py",
            &lines,
            &[function("charge", 1, 3)],
            FileFlags::empty(),
        )?;
        Ok(Self {
            repo,
            orders,
            users,
            billing,
        })
    }

    /// What `prompt` finds, with the graph it is numbered in.
    fn find(&self, prompt: &str) -> TestResult<(LexicalMatch, Graph<NodeId>)> {
        let reader = self.repo.reader()?;
        let graph = read_graph(reader.queries(), &GraphSettings::default())?;
        let found = lexical(reader.queries(), &graph, &PromptTerms::of(prompt))?;
        Ok((found, graph))
    }
}

/// A node's number in `graph`.
fn at(graph: &Graph<NodeId>, node: NodeId) -> TestResult<u32> {
    Ok(graph.index_of(node).ok_or("a node of the store")?)
}

/// Worked by hand with 8 rows: a word in 1 row weighs ln(1 + 7.5 / 1.5) = 1.791759, in 2 rows
/// ln(1 + 6.5 / 2.5) = 1.280934; the floor is twice the first, 3.583519. "cancel the order" looks up
/// `cancel` and `order`: `cancel_order` holds both and ranks first, weighing 1; `create_order` holds
/// `order` and weighs less. `orders.py` holds both words through its symbols: (1.791759 +
/// 1.280934) / 3.583519 = 0.857451, which is also the top lexical score. Nothing else matched.
#[test]
fn a_file_matches_by_the_rare_words_it_holds() -> TestResult {
    let shop = Shop::new()?;
    let (found, graph) = shop.find("cancel the order")?;
    assert_eq!(found.seeds.get(&at(&graph, shop.orders[2])?), Some(&1.0));
    let create = found
        .seeds
        .get(&at(&graph, shop.orders[1])?)
        .copied()
        .unwrap_or(0.0);
    assert!(create > 0.0 && create < 1.0, "{create}");
    assert_eq!(found.seeds.len(), 2);
    let orders = found
        .files
        .get(&at(&graph, shop.orders[0])?)
        .ok_or("orders.py matched")?;
    assert!(
        (orders.coverage - 0.857_451_4).abs() < 1e-6,
        "{}",
        orders.coverage
    );
    assert_eq!(orders.words, ["cancel", "order"]);
    assert!(!orders.is_named);
    assert_eq!(found.files.len(), 1);
    assert!((found.top_lexical - 0.857_451_4).abs() < 1e-6);
    Ok(())
}

/// One rare word alone weighs 1.791759 over the floor of 3.583519: a match of exactly 0.5, so a
/// prompt of one word cannot make a perfect match.
#[test]
fn one_word_alone_matches_at_most_half() -> TestResult {
    let shop = Shop::new()?;
    let (found, graph) = shop.find("charge")?;
    let billing = found
        .files
        .get(&at(&graph, shop.billing[0])?)
        .ok_or("billing.py matched")?;
    assert!(
        (billing.coverage - 0.5).abs() < 1e-9,
        "{}",
        billing.coverage
    );
    assert!((found.top_lexical - 0.5).abs() < 1e-9);
    Ok(())
}

/// A file and a symbol the prompt names exactly are seeds of weight 1, and the file matches fully.
/// The other files match by the words they hold, worked by hand: the prompt's words weigh
/// `create` 1.280934, `user` 1.791759, `shop` (4 rows) ln 2 = 0.693147 and `users` 1.791759,
/// 5.557599 in all, `py` being the extension of the path named, no word; `orders.py` holds `shop`
/// and (through `create_order`) `create`: 1.974081 / 5.557599 = 0.355204; `billing.py` holds
/// `shop`: 0.693147 / 5.557599 = 0.124721.
#[test]
fn what_a_prompt_names_exactly_matches_fully() -> TestResult {
    let shop = Shop::new()?;
    let (found, graph) = shop.find("fix `create_user` in shop/users.py")?;
    assert_eq!(found.seeds.get(&at(&graph, shop.users[0])?), Some(&1.0));
    assert_eq!(found.seeds.get(&at(&graph, shop.users[1])?), Some(&1.0));
    let users = found
        .files
        .get(&at(&graph, shop.users[0])?)
        .ok_or("users.py")?;
    assert!(users.is_named);
    assert_eq!(users.coverage, 1.0);
    assert_eq!(found.top_lexical, 1.0);
    let orders = found
        .files
        .get(&at(&graph, shop.orders[0])?)
        .ok_or("orders.py")?;
    assert!(
        (orders.coverage - 0.355_203_9).abs() < 1e-6,
        "{}",
        orders.coverage
    );
    let billing = found
        .files
        .get(&at(&graph, shop.billing[0])?)
        .ok_or("billing.py")?;
    assert!(
        (billing.coverage - 0.124_720_6).abs() < 1e-6,
        "{}",
        billing.coverage
    );
    Ok(())
}

/// A path with a line names the symbol whose lines hold it: line 8 of `orders.py` is in
/// `cancel_order` (7 to 9). An absolute path, longer than any the repository holds, names the file
/// whose path it ends with, and line 2 of `billing.py` is in `charge`.
#[test]
fn a_path_s_line_names_its_symbol() -> TestResult {
    let shop = Shop::new()?;
    let (found, graph) = shop.find("error at shop/orders.py:8")?;
    assert_eq!(found.seeds.get(&at(&graph, shop.orders[2])?), Some(&1.0));
    assert!(
        found
            .files
            .get(&at(&graph, shop.orders[0])?)
            .is_some_and(|file| file.is_named)
    );
    let (found, graph) = shop.find("see /home/dev/project/shop/billing.py:2 now")?;
    assert_eq!(found.seeds.get(&at(&graph, shop.billing[1])?), Some(&1.0));
    assert!(
        found
            .files
            .get(&at(&graph, shop.billing[0])?)
            .is_some_and(|file| file.is_named)
    );
    Ok(())
}

/// A file the prompt names matches fully even when it holds few of the prompt's words:
/// `orders.py` holds none of `charge`, yet the prompt names it.
#[test]
fn a_named_file_matches_fully_whatever_its_words() -> TestResult {
    let shop = Shop::new()?;
    let (found, graph) = shop.find("move charge into shop/orders.py")?;
    let orders = found
        .files
        .get(&at(&graph, shop.orders[0])?)
        .ok_or("orders.py")?;
    assert!(orders.is_named);
    assert!(!orders.words.contains(&"charge".to_owned()));
    assert_eq!(orders.coverage, 1.0);
    Ok(())
}

/// A prompt of words that name no place finds nothing: no seed, no file, a top score of 0.
#[test]
fn a_vague_prompt_finds_nothing() -> TestResult {
    let shop = Shop::new()?;
    let (found, _) = shop.find("make it better")?;
    assert_eq!(found, LexicalMatch::default());
    Ok(())
}

/// A name six symbols share is no one place: none of them is named, though the word still finds
/// them by BM25.
#[test]
fn a_name_many_symbols_share_names_none() -> TestResult {
    let mut repo = Repo::new()?;
    let lines = "line\n".repeat(3);
    for file in 0..6 {
        repo.file(
            &format!("jobs/job{file}.py"),
            &lines,
            &[function("run", 1, 2)],
            FileFlags::empty(),
        )?;
    }
    let reader = repo.reader()?;
    let graph = read_graph(reader.queries(), &GraphSettings::default())?;
    let found = lexical(reader.queries(), &graph, &PromptTerms::of("`run`"))?;
    assert!(found.files.values().all(|file| !file.is_named));
    assert!(!found.seeds.is_empty());
    Ok(())
}
