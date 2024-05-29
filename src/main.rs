use eyre::Result;
use petgraph::{graph::NodeIndex, visit::EdgeRef, Directed};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::runtime::Runtime;
use petgraph::Graph;
use std::fs::File;
use std::io::{Write, Read};
use std::collections::HashSet;
use std::time::Instant;

#[allow(dead_code, non_snake_case)]
#[derive(Debug, Deserialize)]
struct TransactionResponse {
    status: String,
    message: String,
    result: Vec<RawTransaction>,
}

#[allow(dead_code, non_snake_case)]
#[derive(Debug, Deserialize, Serialize, Clone)]
struct RawTransaction {
    blockHash: String,
    blockNumber: String,
    from: String,
    to: String,
    gas: String,
    gasPrice: String,
    gasUsed: String,
    hash: String,
    value: String,
    nonce: String,
    transactionIndex: String,
    timeStamp: String,
    isError: String,
    txreceipt_status: String,
    input: String,
    contractAddress: String,
    cumulativeGasUsed: String,
    functionName: String,
    methodId: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize, Serialize, Clone)]
struct SimplifiedTransaction {
    hash: String,
    value: String,
    timeStamp: String,
}

#[derive(Serialize, Deserialize)]
struct SerializableGraph {
    nodes: Vec<String>,
    edges: Vec<(usize, usize, SimplifiedTransaction)>,
}

#[derive(Debug, Deserialize)]
struct EthPriceRecord {
    unix_epoch_at_the_start_of_averaging_period: u64,
    average_price_in_usd: f64,
}

type G = Graph<String, SimplifiedTransaction, Directed>;

const TRAVERSAL_STARTING_ADDRESS: &str = "0x4976A4A02f38326660D17bf34b431dC6e2eb2327"; // Binance affiliated address
const MAX_TRANSACTIONS_TO_PARSE: usize = 100_000_000; // Limit of transactions near which parsing will be stopped.
const TRANSACTIONS_TO_REQUEST_FROM_EACH_ADDRESS: usize = 10_000; // Limit of transactions to request (from and to) one particular address, <= 10000
const DATA_STORAGE_FOLDER: &str = "json";

async fn get_transactions_for_address(
    address: &str,
    client: &Client,
    api_key: &String,
) -> Result<TransactionResponse> {
    let start_block = "0";
    let end_block = "99999999";
    let page = "1";
    let sort = "desc";
    let offset = TRANSACTIONS_TO_REQUEST_FROM_EACH_ADDRESS;

    let request_url = format!(
        "https://api.etherscan.io/api?module=account&action=txlist&address={}&startblock={}&endblock={}&page={}&offset={}&sort={}&apikey={}",
        address, start_block, end_block, page, offset, sort, api_key
    );
    let response = client.get(&request_url).send().await?;

    if response.status().is_success() {
        let body_bytes = response.bytes().await?;
        match serde_json::from_slice::<TransactionResponse>(&body_bytes) {
            Ok(parsed_response) => Ok(parsed_response),
            Err(_) => {
                let error_body = String::from_utf8_lossy(&body_bytes);
                Err(eyre::eyre!(
                    "Failed to decode JSON response: {}",
                    error_body
                ))
            }
        }
    } else {
        Err(eyre::eyre!("Response status errored."))
    }
}

async fn graph_data_collection_procedure(
    address_relevance_counter: &mut HashMap<String, u64>,
    blockchain_graph: &mut G,
    node_indices: &mut HashMap<String, NodeIndex>,
    edges: &mut HashMap<String, SimplifiedTransaction>,
    client: &Client,
    api_key: &String,
    address_to_check: String,
) {

    let response = {
        loop {
            let attempt = get_transactions_for_address(&address_to_check, client, api_key).await;
            match attempt {
                Err(e) => {
                    println!("Incorrect response for {}:\n{}", &address_to_check, e);
                }
                Ok(t) => {
                    println!("Correct response for {} with {} transactions", &address_to_check, t.result.len());
                    break t;
                }
            }
        }
    };

    for transaction in response.result.iter() {
        if transaction.contractAddress == "".to_string()
        && transaction.isError == "0"
        && transaction.from != "GENESIS" 
        && !edges.contains_key(&transaction.hash)
        {
            *address_relevance_counter.entry(transaction.to.clone()).or_insert(0) +=1; // Counting to find the best direction to move futher
            *address_relevance_counter.entry(transaction.from.clone()).or_insert(0) +=1; // Counting to find the best direction to move futher 

            let simplified_transacion = SimplifiedTransaction {
                hash: transaction.hash.clone(),
                value: transaction.value.clone(),
                timeStamp: transaction.timeStamp.clone()
            };
        
            let origin = *node_indices
                .entry(transaction.from.clone())
                .or_insert_with(|| {
                    blockchain_graph.add_node(transaction.from.clone())
                });

            let target = *node_indices
                .entry(transaction.to.clone())
                .or_insert_with(|| {
                    blockchain_graph.add_node(transaction.to.clone())
                });

            edges.insert(transaction.hash.clone(), simplified_transacion.clone());
            blockchain_graph.add_edge(origin, target, simplified_transacion);
        }
    }

}

async fn parse_blockchain(traversal_starting_adress: String, api_key: &String) -> Graph<String, SimplifiedTransaction> {
    let client = Client::new();
    let mut blockchain_graph: Graph::<String, SimplifiedTransaction, Directed> = Graph::new();
    let mut node_indices = HashMap::new();
    let mut edges = HashMap::new();

    let mut address_relevance_counter: HashMap<String, u64> = HashMap::from([(traversal_starting_adress.clone().to_lowercase(), 1)]);
    let mut trajectory: Vec<String> = vec![];
    
    loop {
        let current_edge_count = blockchain_graph.edge_count();
        println!("Current transaction count is {} out of {}", current_edge_count, MAX_TRANSACTIONS_TO_PARSE);
        if current_edge_count >= MAX_TRANSACTIONS_TO_PARSE {return blockchain_graph};
        
        let mut counts: Vec<(String, u64)> = address_relevance_counter.clone()
            .into_iter()
            .map(|(k, v)| (k.clone(), v))
            .collect();
            counts.sort_by(|a, b| b.1.cmp(&a.1));
        
        let priority_address = counts
            .iter()
            .map(|(address, _)| address.clone())
            .find(|address| !trajectory.contains(address))
            .unwrap();
        
            trajectory.push(priority_address.clone());

            let future = graph_data_collection_procedure(
                &mut address_relevance_counter,
                &mut blockchain_graph,
                &mut node_indices,
                &mut edges,
                &client, 
                api_key,
                priority_address,
            );
            future.await;
        }
    }
    
fn serialize_graph(graph: &G, pathname: &str) -> Result<()> {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for node in graph.node_indices() {
        nodes.push(graph[node].clone());
    }

    for edge in graph.edge_indices() {
        let (source, target) = graph.edge_endpoints(edge).unwrap();
        edges.push((source.index(), target.index(), graph[edge].clone()));
    }

    let serializable_graph = SerializableGraph { nodes, edges };
    let file_pathname = format!("{}/{}", DATA_STORAGE_FOLDER, pathname);
    let file = File::create(&file_pathname)?;
    serde_json::to_writer_pretty(file, &serializable_graph)?;

    println!("\nSaved graph with {} edges and {} nodes as {}\n", &graph.edge_count(), &graph.node_count(), &file_pathname);
    Ok(())
}


#[allow(dead_code)]
fn deserialize_graph(pathname: &str) -> Result<G> {
    let file_pathname = format!("{}/{}", DATA_STORAGE_FOLDER, pathname);
    let mut json = String::new();
    
    println!("\nTrying to load {}", file_pathname);
    let mut file = File::open(&file_pathname).map_err(|_| eyre::eyre!(format!("File {} not found.", file_pathname)))
    .unwrap();

    file.read_to_string(&mut json).unwrap();

    let serializable_graph: SerializableGraph = serde_json::from_str(&json)?;

    let mut graph = Graph::new();
    let mut node_indices = Vec::new();

    for node in serializable_graph.nodes {
        node_indices.push(graph.add_node(node));
    }

    for (source, target, weight) in serializable_graph.edges {
        graph.add_edge(node_indices[source], node_indices[target], weight);
    }

    Ok(graph)
}

fn get_api_key() -> String {
    let mut api_key: String = String::new();
    File::open("api_key.txt")
        .map_err(|_| eyre::eyre!("Please provide an Etherscan API key (put it inside api_key.txt)"))
        .unwrap()
        .read_to_string(&mut api_key).unwrap();
    api_key = api_key.trim().to_string();
    assert_ne!(api_key, "");
    api_key
}

fn filter_twoway_edges(graph: &G) -> G {
    let mut filtered_graph = graph.clone();
    filtered_graph.clear_edges();

    for edge in graph.edge_references() {
        let (source, target) = (edge.source(), edge.target());
        if graph.find_edge(target, source).is_some() {
            let transaction = edge.weight().clone();
            filtered_graph.add_edge(source, target, transaction);
        }
    }

    filtered_graph
}

fn calculate_two_way_flow(graph: &G, prices: &Vec<EthPriceRecord>) -> (f64, f64, f64, String) {
    let mut detailed_log = String::new();
    let mut total_volume_usd = 0.0;
    let mut total_flow_usd = 0.0;

    let mut visited_pairs = HashSet::new();
    for first_edge in graph.edge_references() {
        let node_a = first_edge.source();
        let node_b = first_edge.target();

        if visited_pairs.contains(&(node_a, node_b)) {continue}

        let mut pair_volume_usd = 0.0;
        let mut sum_a_to_b_usd = 0.0;
        let mut sum_b_to_a_usd = 0.0;
        
        let edges_a_to_b = graph.edges_connecting(node_a, node_b).count();
        let edges_b_to_a = graph.edges_connecting(node_b, node_a).count();
        let edges_info: String = if node_a != node_b {
                format!("{} + {}", edges_a_to_b, edges_b_to_a)
            } else {             
                format!("{} self", edges_a_to_b)
            };
        
        detailed_log.push_str(&format!(
            "Two-way transaction set for addresses {:?} <-> {:?} ({}):\n",
            &graph[node_a], &graph[node_b], &edges_info
        ));

        for edge in graph.edges(node_a) {
            if edge.target() == node_b {
                let volume_wei: f64 = edge.weight().value.parse().unwrap();
                let timestamp: u64 = edge.weight().timeStamp.parse().unwrap();
                let volume_in_usd = (volume_wei / 1e18) * get_price_at_timestamp(timestamp, prices);
                pair_volume_usd += volume_in_usd;
                sum_a_to_b_usd += volume_in_usd;
                detailed_log.push_str(&format!("      |-> hash: {} at {} unix epoch, volume: {:.0} USD\n", &edge.weight().hash, timestamp, volume_in_usd));
            }
        }

        if node_a != node_b { 
            for edge in graph.edges(node_b) {
                if edge.target() == node_a {
                    let volume_wei: f64 = edge.weight().value.parse().unwrap();
                    let timestamp: u64 = edge.weight().timeStamp.parse().unwrap();
                    let volume_in_usd = (volume_wei / 1e18) * get_price_at_timestamp(timestamp, prices);
                    pair_volume_usd += volume_in_usd;
                    sum_b_to_a_usd += volume_in_usd;
                    detailed_log.push_str(&format!("      <-| hash: {} at {} unix epoch, volume: {:.0} USD\n", &edge.weight().hash, timestamp, volume_in_usd));
                }
            }
        } 
        let pair_flow_usd = if node_a != node_b {
            (sum_a_to_b_usd - sum_b_to_a_usd).abs()
        } else {
            0.0
        };
        total_volume_usd += pair_volume_usd;
        total_flow_usd += pair_flow_usd;

        visited_pairs.insert((node_a, node_b));
        visited_pairs.insert((node_b, node_a));

        detailed_log.push_str(&format!(
            "Volume for this set: {:.0} USD, Flow for this set: {:.0} USD\n\n",
            pair_volume_usd, pair_flow_usd
        ));

    }

    detailed_log.push_str(&format!("Total volume: {:.0} USD\n", total_volume_usd));
    detailed_log.push_str(&format!("Total flow: {:.0} USD\n", total_flow_usd));

    (total_volume_usd, total_volume_usd/graph.edge_count() as f64, total_flow_usd, detailed_log)
}


fn get_eth_hourly_prices(file_path: &str) -> Result<Vec<EthPriceRecord>> {
    let mut reader = csv::Reader::from_path(file_path).unwrap();
    let mut records = Vec::new();

    for result in reader.records() {
        let record = result.unwrap();
        let unix_epoch_at_the_start_of_averaging_period: u64 = record[0].parse::<f64>().unwrap().round() as u64;
        let average_price_in_usd: f64 = record[1].parse().unwrap();

        records.push(EthPriceRecord {
            unix_epoch_at_the_start_of_averaging_period,
            average_price_in_usd,
        });
    }

    Ok(records)
}

fn get_price_at_timestamp(timestamp: u64, prices: &Vec<EthPriceRecord>) -> f64 {
    let maybe_price = prices.iter().find(|&price| {
        let period_start = price.unix_epoch_at_the_start_of_averaging_period as u64;
        let period_end = period_start + 3600;
        period_start <= timestamp && timestamp < period_end
    }).map(|price| price.average_price_in_usd);

    match maybe_price {
        Some(price) => price,
        None => {
            let last_record = prices.iter()
            .max_by(|record_a, record_b| record_a.unix_epoch_at_the_start_of_averaging_period.cmp(&record_b.unix_epoch_at_the_start_of_averaging_period));
            let last_timestamp = last_record.unwrap().unix_epoch_at_the_start_of_averaging_period;
            let last_price = last_record.unwrap().average_price_in_usd;

            if timestamp > last_timestamp {last_price} else {panic!("No price found")}
        }
    }

}

fn filter_by_transaction_price(graph: &G, prices: &Vec<EthPriceRecord>, lower_usd_bound: f64, higher_usd_bound: f64) -> G {
    let mut filtered_graph = graph.clone();
    filtered_graph.clear_edges();

    for edge in graph.edge_references() {
        let transaction = edge.weight();
        let timestamp: u64 = transaction.timeStamp.parse().unwrap();
        let eth_price = get_price_at_timestamp(timestamp, prices);
        let transaction_value_in_usd = (transaction.value.parse::<f64>().unwrap() / 1e18) * eth_price;
        if lower_usd_bound <= transaction_value_in_usd && transaction_value_in_usd <= higher_usd_bound {
            filtered_graph.add_edge(edge.source(), edge.target(), transaction.clone());
        }
    }

    filtered_graph
}

fn calculate_total_usd_volume(graph: &G, prices: &Vec<EthPriceRecord>) -> (f64, f64) {
    let mut total_volume_usd = 0.0;

    for edge in graph.edge_references() {
        let transaction = edge.weight();
        let timestamp: u64 = transaction.timeStamp.parse().unwrap();
        let eth_price = get_price_at_timestamp(timestamp, prices);
        let transaction_value_in_usd = (transaction.value.parse::<f64>().unwrap() / 1e18) * eth_price;
        total_volume_usd += transaction_value_in_usd;
    }
    let mean_value_usd = total_volume_usd / graph.edge_count() as f64;

    (total_volume_usd, mean_value_usd)
}

#[test]
fn test_main() ->Result<(), ()> {  
    let graph = deserialize_graph("handcrafted_for_testing.json").unwrap();
    let prices = get_eth_hourly_prices("eth_prices.csv").unwrap();

    let (graph_volume, graph_mean) = calculate_total_usd_volume(&graph, &prices);
    assert_eq!(graph_volume.ceil(), 21011.0);
    assert_eq!(graph_mean.ceil(), 2627.0);
    assert_eq!(graph.edge_count(), 8);

    // Price filtered graph
    let usd_lower_bound = 10.0;
    let usd_higher_bound = 1000.0;
    let price_filtered_graph = filter_by_transaction_price(&graph, &prices, usd_lower_bound, usd_higher_bound);
    let (price_filtered_graph_volume, price_filtered_graph_mean) = calculate_total_usd_volume(&price_filtered_graph, &prices);
    assert_eq!(price_filtered_graph_volume.ceil(), 1127.0);
    assert_eq!(price_filtered_graph_mean.ceil(), 564.0);
    assert_eq!(price_filtered_graph.edge_count(), 2);
    
    // Two-way filtered graph
    let twoway_filtered_graph = filter_twoway_edges(&graph);
    let (twoway_filtered_graph_volume, twoway_filtered_graph_mean_value, twoway_filtered_graph_flow, _) = calculate_two_way_flow(&twoway_filtered_graph, &prices);
    assert_eq!(twoway_filtered_graph_volume.ceil(), 12009.0) ;
    assert_eq!(twoway_filtered_graph_mean_value.ceil(), 2002.0);
    assert_eq!(twoway_filtered_graph_flow.ceil(), 3755.0);
    assert_eq!(twoway_filtered_graph.edge_count(), 6);
    
    // Two-way and price filtered graph
    let twoway_price_filtered_graph = filter_twoway_edges(&price_filtered_graph);
    let (twoway_price_filtered_graph_volume, _, twoway_price_filtered_graph_flow, _) = calculate_two_way_flow(&twoway_price_filtered_graph, &prices);
    assert_eq!(twoway_price_filtered_graph_volume, 0.0);
    assert_eq!(twoway_price_filtered_graph_flow, 0.0);
    assert_eq!(twoway_price_filtered_graph.edge_count(), 0);
    
    Ok(())
}

fn main() {  
    let async_timer: Instant = Instant::now();
    let api_key = get_api_key();
    let rt = Runtime::new().unwrap();
    let graph = rt.block_on(parse_blockchain(TRAVERSAL_STARTING_ADDRESS.to_string(), &api_key));

    println!("Async operations took {:.3} s", async_timer.elapsed().as_secs_f64());
    let timer: Instant = Instant::now();

    let mut result_log = String::new();

    let _ = serialize_graph(&graph, "parsed.json");

    let prices = get_eth_hourly_prices("eth_prices.csv").unwrap();

    let (graph_volume, graph_mean) = calculate_total_usd_volume(&graph, &prices);
    let s = format!(
        "For all parsed transactions:\nTotal volume: {:.0} USD, Mean value: {:.0} USD, N: {}\n\n",
        graph_volume, graph_mean, graph.edge_count()
    );
    print!("{}", &s);
    result_log.push_str(&s);

    // Price filtered graph
    let usd_lower_bound = 10.0;
    let usd_higher_bound = 1000.0;
    let price_filtered_graph = filter_by_transaction_price(&graph, &prices, usd_lower_bound, usd_higher_bound);
    let (price_filtered_graph_volume, price_filtered_graph_mean) = calculate_total_usd_volume(&price_filtered_graph, &prices);
    let s = format!(
        "For transactions in {}-{} USD range:\nTotal volume: {:.0} USD, Mean value: {:.0} USD, N: {}\n\n",
        usd_lower_bound, usd_higher_bound, price_filtered_graph_volume, price_filtered_graph_mean, price_filtered_graph.edge_count()
    );
    print!("{}", &s);
    result_log.push_str(&s);   

    // Two-way filtered graph
    let twoway_filtered_graph = filter_twoway_edges(&graph);
    let (twoway_filtered_graph_volume, twoway_filtered_graph_mean_value, twoway_filtered_graph_flow, twoway_filtered_graph_logs) = calculate_two_way_flow(&twoway_filtered_graph, &prices);
    let s = format!(
        "For two-way transactions: \nTotal volume: {:.0} USD, Mean value: {:.0} USD, Total flow: {:.0} USD, N: {}\n\n",
        twoway_filtered_graph_volume, twoway_filtered_graph_mean_value, twoway_filtered_graph_flow, twoway_filtered_graph.edge_count()
    );
    print!("{}", &s);
    result_log.push_str(&s);
    
    // Two-way and price filtered graph
    let twoway_price_filtered_graph = filter_twoway_edges(&price_filtered_graph);
    let (twoway_price_filtered_graph_volume, twoway_price_filtered_graph_mean_value, twoway_price_filtered_graph_flow, twoway_price_filtered_graph_logs) = calculate_two_way_flow(&twoway_price_filtered_graph, &prices);
    let s = format!(
        "For two-way transactions in {}-{} USD range: \nTotal volume: {:.0} USD, Mean value: {:.0} USD, Total flow: {:.0} USD, N: {}\n\n",
        usd_lower_bound, usd_higher_bound, twoway_price_filtered_graph_volume, twoway_price_filtered_graph_mean_value, twoway_price_filtered_graph_flow, twoway_price_filtered_graph.edge_count()
    );
    print!("{}", &s);
    result_log.push_str(&s);

    let mut log_file_main= File::create("result.txt").unwrap();
    write!(log_file_main, "{}", result_log).unwrap();

    let mut log_file_twoway = File::create("twoway_filtered_graph_logs.txt").unwrap();    
    let mut log_file_twoway_price = File::create("twoway_price_filtered_graph_logs.txt").unwrap();    
    let twoway_filtered_graph_logs = format!("Two-way transactions in {}-{} USD range detailed logs:\n{}", usd_lower_bound, usd_higher_bound, twoway_filtered_graph_logs);
    let twoway_price_filtered_graph_logs = format!("Two-way transactions detailed logs:\n{}", twoway_price_filtered_graph_logs);
    log_file_twoway.write_all(twoway_filtered_graph_logs.as_bytes()).unwrap();
    log_file_twoway_price.write_all(twoway_price_filtered_graph_logs.as_bytes()).unwrap();

    println!("Local operations took {:.3} s", timer.elapsed().as_secs_f64());
    println!("Local + async operations took {:.3} s", async_timer.elapsed().as_secs_f64());
}
