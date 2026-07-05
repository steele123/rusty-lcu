use rusty_lcu::generated::{
    ENDPOINTS, TAGS, endpoints_for_tag, find_endpoint, find_endpoint_by_operation_id,
};

fn main() {
    println!("generated endpoints: {}", ENDPOINTS.len());
    println!("generated tags: {}", TAGS.len());
    println!(
        "typed request endpoints: {}",
        ENDPOINTS
            .iter()
            .filter(|endpoint| endpoint.request_type.is_some())
            .count()
    );
    println!(
        "typed response endpoints: {}",
        ENDPOINTS
            .iter()
            .filter(|endpoint| endpoint.response_type.is_some())
            .count()
    );

    if let Some(endpoint) = find_endpoint("GET", "/lol-summoner/v1/current-summoner") {
        println!(
            "{} {} -> {:?}",
            endpoint.method, endpoint.path, endpoint.response_type
        );
    }

    if let Some(endpoint) = find_endpoint_by_operation_id("GetLolGameflowV1GameflowPhase") {
        println!("operation lookup: {} {}", endpoint.method, endpoint.path);
    }

    let summoner_count = endpoints_for_tag("Plugin lol-summoner").count();
    println!("lol-summoner endpoints: {summoner_count}");
}
