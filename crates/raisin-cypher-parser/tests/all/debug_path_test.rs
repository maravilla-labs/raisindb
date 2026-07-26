// SPDX-License-Identifier: BSL-1.1

use raisin_cypher_parser::parse_path;

#[test]
fn debug_variable_length_path() {
    let input = "(start)-[:REL*1..5]->(end)";
    println!("Input: {}", input);
    println!("Length: {}", input.len());
    println!("Char at 23: {:?}", input.chars().nth(23));

    let result = parse_path(input);
    match result {
        Ok(path) => println!("Success: {:?}", path),
        Err(e) => println!("Error: {}", e),
    }

    // Try without the range first
    let simple = "(start)-[:REL]->(end)";
    let result2 = parse_path(simple);
    match result2 {
        Ok(path) => println!("Simple success: {:?}", path),
        Err(e) => println!("Simple error: {}", e),
    }
}
