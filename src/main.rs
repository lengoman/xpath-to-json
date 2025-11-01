use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use anyhow::{Result, Context};
use scraper::{Html, Selector};
use encoding_rs::{Encoding, UTF_8};
use chrono::Datelike;
use regex;

#[derive(Parser)]
#[command(name = "xpath-to-json")]
#[command(about = "A CLI tool that processes HTML using XPath configurations to extract JSON data")]
struct Cli {
    /// Path to the JSON configuration file
    #[arg(long)]
    xpath_config: PathBuf,
    
    /// Path to the HTML file to process
    #[arg(long)]
    html: PathBuf,
    
    /// Path to the output file (optional - if not provided, output will be displayed)
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
struct XPathConfig {
    /// Name of the configuration
    name: String,
    /// Description of what this configuration extracts
    description: Option<String>,
    /// Sample of expected output format
    output_sample: Option<Vec<serde_json::Value>>,
    /// The XPath rules to execute
    rules: Vec<XPathRule>,
}

#[derive(Debug, Deserialize, Serialize)]
struct XPathRule {
    /// Name/identifier for this rule
    name: String,
    /// The XPath expression to execute
    xpath: String,
    /// What type of data to extract (text, attribute, html, etc.)
    extract_type: ExtractType,
    /// Optional attribute name if extracting attributes
    attribute: Option<String>,
    /// Whether this rule should be executed for each item from a previous rule
    iterate_over: Option<String>,
    /// Child rules to execute for each iteration
    children: Option<Vec<XPathRule>>,
    /// Child rules to execute for each iteration (alias for children)
    #[serde(alias = "fields")]
    fields: Option<Vec<XPathRule>>,
    /// For each item rule (new structure)
    #[serde(rename = "for-each-item")]
    for_each_item: Option<Box<XPathRule>>,
    /// Map item rule (new structure)
    #[serde(rename = "map-item")]
    map_item: Option<Box<XPathRule>>,
}

#[derive(Debug, Deserialize, Serialize)]
enum ExtractType {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "attribute")]
    Attribute,
    #[serde(rename = "html")]
    Html,
    #[serde(rename = "count")]
    Count,
    #[serde(rename = "object")]
    Object,
}

#[derive(Debug, Serialize)]
struct ExtractionResult {
    /// The name of the configuration
    config_name: String,
    /// The extracted data
    data: Value,
    /// Any errors that occurred during extraction
    errors: Vec<String>,
}


fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Read and parse the configuration
    let config_content = fs::read_to_string(&cli.xpath_config)
        .context("Failed to read configuration file")?;
    let config: XPathConfig = serde_json::from_str(&config_content)
        .context("Failed to parse configuration JSON")?;
    
    // Read the HTML content with encoding detection
    let html_content = read_html_file(&cli.html)?;
    
    // Process the HTML with the configuration
    let result = process_html(&config, &html_content)?;
    
    // Output the result
    let output_json = serde_json::to_string_pretty(&result)
        .context("Failed to serialize result to JSON")?;
    
    if let Some(output_path) = cli.output {
        fs::write(&output_path, output_json)
            .context("Failed to write output file")?;
        println!("Results written to {:?}", output_path);
    } else {
        println!("{}", output_json);
    }
    
    Ok(())
}

fn read_html_file(path: &PathBuf) -> Result<String> {
    // Read the file as bytes first
    let bytes = fs::read(path)
        .context("Failed to read HTML file")?;
    
    // Try to detect encoding from HTML meta tag
    let html_str = String::from_utf8_lossy(&bytes);
    let encoding = detect_encoding(&html_str);
    
    // Decode using the detected encoding
    let (decoded, _, _) = encoding.decode(&bytes);
    
    Ok(decoded.to_string())
}

fn detect_encoding(html: &str) -> &'static Encoding {
    // Look for charset in meta tag
    if let Some(charset_start) = html.find("charset=") {
        let charset_value = &html[charset_start + 8..];
        let charset_end = charset_value.find(|c: char| c == '"' || c == '>' || c == ' ').unwrap_or(charset_value.len());
        let charset = charset_value[..charset_end].trim().to_lowercase();
        
        match charset.as_str() {
            "utf-8" | "utf8" => return UTF_8,
            "windows-1252" | "iso-8859-1" | "latin1" => {
                return encoding_rs::WINDOWS_1252;
            }
            _ => {
                // Default to UTF-8 if we can't detect
                return UTF_8;
            }
        }
    }
    
    // Default to UTF-8 if no charset found
    UTF_8
}

fn process_html(config: &XPathConfig, html_content: &str) -> Result<ExtractionResult> {
    let mut errors = Vec::new();
    let mut raw_data = serde_json::Map::new();
    
    // Parse HTML
    let document = Html::parse_document(html_content);
    
    // Process each rule to get raw data
    for rule in &config.rules {
        match process_rule(&document, rule) {
            Ok(value) => {
                // Handle nested structure for months -> days -> stocks
                if rule.name == "months" && rule.for_each_item.is_some() {
                    // Process the months XPath directly to get the actual month names
                    let css_selector = xpath_to_css_selector(&rule.xpath)?;
                    let selector = scraper::Selector::parse(&css_selector)
                        .map_err(|e| anyhow::anyhow!("Invalid CSS selector: {}", e))?;
                    let mut months_results = Vec::new();
                    for element in document.select(&selector) {
                        let text = element.text().collect::<String>().trim().to_string();
                        if !text.is_empty() {
                            months_results.push(Value::String(text));
                        }
                    }
                    let months_result = if months_results.len() == 1 {
                        months_results.into_iter().next().unwrap_or(Value::Null)
                    } else {
                        Value::Array(months_results)
                    };
                    raw_data.insert("months".to_string(), months_result);

                    // Process the days for each month
                    if let Some(for_each_item) = &rule.for_each_item {
                        let days_result = process_rule(&document, for_each_item)?;
                        raw_data.insert("days".to_string(), days_result.clone());

                        // Process items for each day using find_items_for_day
                        if let Some(_map_item) = &for_each_item.map_item {
                            // Get the days array
                            let days_array = match &days_result {
                                Value::Array(arr) => arr,
                                _ => return Err(anyhow::anyhow!("Days result is not an array")),
                            };

                            // Create a map of day -> items using find_items_for_day
                            let mut day_items_map = serde_json::Map::new();
                            for day_value in days_array {
                                if let Some(day_str) = day_value.as_str() {
                                    let day_items = find_items_for_day(&document, day_str)?;
                                    day_items_map.insert(day_str.to_string(), Value::Array(day_items));
                                }
                            }
                            
                            // Store the day-items mapping
                            raw_data.insert("day_items".to_string(), Value::Object(day_items_map));
                        }
                    }
                } else {
                    raw_data.insert(rule.name.clone(), value);
                }
            }
            Err(e) => {
                let error_msg = format!("Error processing rule '{}': {}", rule.name, e);
                errors.push(error_msg);
                raw_data.insert(rule.name.clone(), Value::Null);
            }
        }
    }
    
    // Generate structured output based on the configuration
    let structured_data = if let Some(output_sample) = &config.output_sample {
        generate_structured_output(&raw_data, output_sample, &document)?
    } else {
        Value::Object(raw_data)
    };
    
    Ok(ExtractionResult {
        config_name: config.name.clone(),
        data: structured_data,
        errors,
    })
}

fn generate_structured_output(raw_data: &serde_json::Map<String, Value>, output_sample: &[serde_json::Value], document: &Html) -> Result<Value> {
    // Process the hierarchical template structure
    let result = process_hierarchical_template(&output_sample[0], raw_data, document)?;
    Ok(Value::Array(vec![result]))
}

fn process_hierarchical_template(template: &Value, raw_data: &serde_json::Map<String, Value>, document: &Html) -> Result<Value> {
    match template {
        Value::Object(obj) => {
            let mut result = serde_json::Map::new();
            for (key, value) in obj {
                // Special handling for months variable - process BEFORE calling process_template_variable
                if key == "{months}" {
                    // Get the actual month names from the months array
                    if let Some(months_array) = raw_data.get("months").and_then(|v| v.as_array()) {
                        // Sort months in chronological order
                        let mut sorted_months = months_array.clone();
                        sorted_months.sort_by(|a, b| {
                            let month_order = ["January", "February", "March", "April", "May", "June", 
                                             "July", "August", "September", "October", "November", "December"];
                            
                            let a_name = a.as_str().and_then(|s| s.split_whitespace().next()).unwrap_or("");
                            let b_name = b.as_str().and_then(|s| s.split_whitespace().next()).unwrap_or("");
                            
                            let a_index = month_order.iter().position(|&m| m == a_name).unwrap_or(12);
                            let b_index = month_order.iter().position(|&m| m == b_name).unwrap_or(12);
                            
                            a_index.cmp(&b_index)
                        });
                        
                        // Process each month found in the HTML in chronological order
                        let mut month_results = Vec::new();
                        for month_value in sorted_months {
                            if let Some(month_str) = month_value.as_str() {
                                // Extract month name from string like "October 2025     — Ex-Dividend Calendar"
                                let month_name = month_str.split_whitespace().next().unwrap_or("October");
                                let full_month_name = format!("{} 2025", month_name);
                                
                                // Create month-specific raw_data for this month
                                let mut month_raw_data = raw_data.clone();
                                
                                // Process days for this specific month
                                if let Some(day_items_obj) = raw_data.get("day_items").and_then(|v| v.as_object()) {
                                    // Filter day_items to only include items for this month
                                    let mut month_day_items = serde_json::Map::new();
                                    
                                    // For each day, get items using the month-aware function
                                    for (day_key, _) in day_items_obj {
                                        if let Ok(month_items) = find_items_for_day_in_month(&document, day_key, Some(&full_month_name)) {
                                            month_day_items.insert(day_key.clone(), Value::Array(month_items));
                                        }
                                    }
                                    
                                    month_raw_data.insert("day_items".to_string(), Value::Object(month_day_items));
                                }
                                
                                let processed_value = process_hierarchical_template(value, &month_raw_data, document)?;
                                month_results.push((month_name.to_string(), processed_value));
                            }
                        }
                        
                        // Insert months in the correct order
                        for (month_name, processed_value) in month_results {
                            result.insert(month_name, processed_value);
                        }
                        continue;
                    }
                }
                
                // Special handling for numbered day variables like {days0}, {days1}, {days0-30}
                if key.starts_with("{days") && key.ends_with("}") {
                    let rule_name = &key[1..key.len()-1]; // Remove { and }
                    if rule_name.starts_with("days") && rule_name.len() > 4 {
                        // Handle range syntax like "days0-30"
                        if rule_name.contains("-") {
                            if let Some((start_str, end_str)) = rule_name[4..].split_once("-") {
                                if let (Ok(start), Ok(end)) = (start_str.parse::<usize>(), end_str.parse::<usize>()) {
                                    let processed_value = process_day_range_with_items(raw_data, start, end)?;
                                    // Don't use processed_key here, iterate through the result
                                    if let Value::Object(day_map) = processed_value {
                                        for (day_num, items) in day_map {
                                            result.insert(day_num, items);
                                        }
                                    }
                                    continue;
                                }
                            }
                        } else {
                            // Handle single day syntax like "days0"
                            if let Ok(day_index) = rule_name[4..].parse::<usize>() {
                                let processed_value = process_numbered_days_with_items(raw_data, day_index)?;
                                let processed_key = process_template_variable(key, raw_data)?;
                                result.insert(processed_key, processed_value);
                                continue;
                            }
                        }
                    }
                }
                
                    let processed_key = process_template_variable(key, raw_data)?;
                    let processed_value = process_hierarchical_template(value, raw_data, document)?;
                    result.insert(processed_key, processed_value);
            }
            Ok(Value::Object(result))
        },
        Value::Array(arr) => {
            // Special handling for days array - group items by day
            if arr.len() == 1 && arr[0].as_str() == Some("{items}") {
                return process_days_with_items(raw_data);
            }
            
            // Special handling for paired data like {"{history-date}": "{history-value}"}
            if arr.len() == 1 {
                if let Some(obj) = arr[0].as_object() {
                    if obj.len() == 1 {
                        let (key, value) = obj.iter().next().unwrap();
                        if key.starts_with('{') && key.ends_with('}') && 
                           value.as_str().map_or(false, |s| s.starts_with('{') && s.ends_with('}')) {
                            return process_paired_data(key, value.as_str().unwrap(), raw_data);
                        }
                    }
                }
            }
            
            let mut result = Vec::new();
            for item in arr {
                let processed_item = process_hierarchical_template(item, raw_data, document)?;
                result.push(processed_item);
            }
            Ok(Value::Array(result))
        },
        Value::String(s) => {
            if s.starts_with('{') && s.ends_with('}') {
                let rule_name = &s[1..s.len()-1]; // Remove { and }
                
                // Handle special syntactic sugar variables
                if rule_name == "currentYear" {
                    return Ok(Value::String(chrono::Utc::now().year().to_string()));
                } else if rule_name == "currentMonth" {
                    return Ok(Value::String(chrono::Utc::now().month().to_string()));
                } else if rule_name == "currentDay" {
                    return Ok(Value::String(chrono::Utc::now().day().to_string()));
                } else if rule_name == "currentDate" {
                    return Ok(Value::String(chrono::Utc::now().format("%Y-%m-%d").to_string()));
                } else if rule_name.starts_with("days") && rule_name.len() > 4 {
                    // Handle numbered day variables like {days0}, {days1}, etc.
                    if let Ok(day_index) = rule_name[4..].parse::<usize>() {
                        return process_numbered_days_with_items(raw_data, day_index);
                    }
                } else {
                    // Handle regular rule variables
                    if let Some(raw_value) = raw_data.get(rule_name) {
                        return Ok(raw_value.clone());
                    }
                }
            }
            Ok(Value::String(s.clone()))
        },
        _ => Ok(template.clone())
    }
}

fn process_paired_data(key_template: &str, value_template: &str, raw_data: &serde_json::Map<String, Value>) -> Result<Value> {
    // Extract rule names from templates
    let key_rule = &key_template[1..key_template.len()-1]; // Remove { and }
    let value_rule = &value_template[1..value_template.len()-1]; // Remove { and }
    
    // Get the arrays for both rules
    let empty_vec = Vec::new();
    let key_array = raw_data.get(key_rule).and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    let value_array = raw_data.get(value_rule).and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    
    let mut result = Vec::new();
    
    // Create pairs up to the minimum length
    let min_len = key_array.len().min(value_array.len());
    for i in 0..min_len {
        if let (Some(key_val), Some(value_val)) = (key_array.get(i), value_array.get(i)) {
            if let (Some(key_str), Some(value_str)) = (key_val.as_str(), value_val.as_str()) {
                let mut pair = serde_json::Map::new();
                pair.insert(key_str.trim().to_string(), Value::String(value_str.trim().to_string()));
                result.push(Value::Object(pair));
            }
        }
    }
    
    Ok(Value::Array(result))
}

fn process_days_with_items(raw_data: &serde_json::Map<String, Value>) -> Result<Value> {
    // Get days and items arrays (generic field names)
    let empty_vec = vec![];
    let days = raw_data.get("days").and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    let items = raw_data.get("items").and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    
    // Group items by day
    let mut result = serde_json::Map::new();
    
    // Create a map of day -> items
    let mut day_items_map: std::collections::HashMap<String, Vec<Value>> = std::collections::HashMap::new();
    
    // For each item, try to find which day it belongs to
    // This is a simplified approach - in reality, you might need more complex logic
    // to properly associate items with specific days based on the HTML structure
    
    for (i, item) in items.iter().enumerate() {
        // Use modulo to distribute items across available days
        if !days.is_empty() {
            let day_index = i % days.len();
            if let Some(day_value) = days.get(day_index) {
                if let Some(day_str) = day_value.as_str() {
                    let day_key = day_str.trim().to_string();
                    day_items_map.entry(day_key).or_insert_with(Vec::new).push(item.clone());
                }
            }
        }
    }
    
    // Convert the map to the result structure
    for (day, items_for_day) in day_items_map {
        result.insert(day, Value::Array(items_for_day));
    }
    
    Ok(Value::Object(result))
}

fn process_numbered_days_with_items(raw_data: &serde_json::Map<String, Value>, day_index: usize) -> Result<Value> {
    // Get days and items arrays (generic field names)
    let empty_vec = vec![];
    let days = raw_data.get("days").and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    let items = raw_data.get("items").and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    
    // Get the specific day for this index
    let day_value = if day_index < days.len() {
        days.get(day_index)
    } else {
        None
    };
    
    if let Some(day) = day_value {
        if let Some(day_str) = day.as_str() {
            let _day_key = day_str.trim().to_string();
            
            // Get items for this specific day
            // This is a simplified approach - you might need more complex logic
            // to properly associate items with specific days based on the HTML structure
            let mut items_for_day = Vec::new();
            
            // For now, distribute items evenly across days
            if !days.is_empty() && !items.is_empty() {
                let items_per_day = items.len() / days.len();
                let start_index = day_index * items_per_day;
                let end_index = if day_index == days.len() - 1 {
                    items.len()
                } else {
                    start_index + items_per_day
                };
                
                for i in start_index..end_index {
                    if let Some(item) = items.get(i) {
                        items_for_day.push(item.clone());
                    }
                }
            }
            
            return Ok(Value::Array(items_for_day));
        }
    }
    
    Ok(Value::Array(vec![]))
}

fn process_day_range_with_items(raw_data: &serde_json::Map<String, Value>, start_day: usize, end_day: usize) -> Result<Value> {
    let mut result = serde_json::Map::new();
    
    // Use the new day_items mapping if available
    if let Some(day_items_obj) = raw_data.get("day_items")
        .and_then(|v| v.as_object()) {
        
        for i in start_day..=end_day {
            // Skip day 0 as requested by user
            if i == 0 {
                continue;
            }
            let day_key = i.to_string();
            if let Some(day_items) = day_items_obj.get(&day_key) {
                result.insert(day_key, day_items.clone());
            } else {
                result.insert(day_key, Value::Array(Vec::new()));
            }
        }
        
        return Ok(Value::Object(result));
    }
    
    // Fallback to old method if day_items not available
    let empty_vec = vec![];
    let days = raw_data.get("days").and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    let items = raw_data.get("items").and_then(|v| v.as_array()).unwrap_or(&empty_vec);
    
    if !days.is_empty() && !items.is_empty() {
        let items_per_day = items.len() / days.len();
        
        for (day_index, day_value) in days.iter().enumerate() {
            if let Some(day_str) = day_value.as_str() {
                let day_key = day_str.trim().to_string();
                
                // Get items for this specific day
                let mut items_for_day = Vec::new();
                
                let start_index = day_index * items_per_day;
                let end_index = if day_index == days.len() - 1 {
                    items.len()
                } else {
                    start_index + items_per_day
                };
                
                for i in start_index..end_index {
                    if let Some(item) = items.get(i) {
                        items_for_day.push(item.clone());
                    }
                }
                
                result.insert(day_key, Value::Array(items_for_day));
            }
        }
    }
    
    Ok(Value::Object(result))
}

fn process_template_variable(key: &str, raw_data: &serde_json::Map<String, Value>) -> Result<String> {
    if key.starts_with('{') && key.ends_with('}') {
        let rule_name = &key[1..key.len()-1]; // Remove { and }
        
        // Handle special syntactic sugar variables
        if rule_name == "currentYear" {
            return Ok(chrono::Utc::now().year().to_string());
        } else if rule_name == "currentMonth" {
            return Ok(chrono::Utc::now().month().to_string());
        } else if rule_name == "currentDay" {
            return Ok(chrono::Utc::now().day().to_string());
        } else if rule_name == "currentDate" {
            return Ok(chrono::Utc::now().format("%Y-%m-%d").to_string());
        } else if rule_name == "months" {
            // Handle months variable - extract month names from the months array
            if let Some(months_array) = raw_data.get("months").and_then(|v| v.as_array()) {
                if let Some(first_month) = months_array.first() {
                    if let Some(month_str) = first_month.as_str() {
                        // Extract month name from string like "October 2025     — Ex-Dividend Calendar"
                        let month_name = month_str.split_whitespace().next().unwrap_or("October");
                        return Ok(month_name.to_string());
                    }
                }
            }
        } else if rule_name.starts_with("days") && rule_name.len() > 4 {
            // Handle numbered day variables like {days0}, {days1}, etc.
            if rule_name.contains("-") {
                // Handle range syntax like "days0-30" - return the range as is for key processing
                return Ok(rule_name.to_string());
            } else if let Ok(day_index) = rule_name[4..].parse::<usize>() {
                if let Some(days_array) = raw_data.get("days").and_then(|v| v.as_array()) {
                    if let Some(day_value) = days_array.get(day_index) {
                        if let Some(day_str) = day_value.as_str() {
                            return Ok(day_str.trim().to_string());
                        }
                    }
                }
            }
        } else {
            // Handle regular rule variables - use the first value if it's an array
            if let Some(raw_value) = raw_data.get(rule_name) {
                if let Some(raw_array) = raw_value.as_array() {
                    if let Some(first_value) = raw_array.first() {
                        if let Some(str_value) = first_value.as_str() {
                            return Ok(str_value.trim().to_string());
                        }
                    }
                } else if let Some(str_value) = raw_value.as_str() {
                    return Ok(str_value.trim().to_string());
                }
            }
        }
    }
    
    Ok(key.to_string())
}

fn process_rule(document: &Html, rule: &XPathRule) -> Result<Value> {
    // Handle nested structure with for-each-item and map-item
    if let Some(for_each_item) = &rule.for_each_item {
        // Process the for-each-item rule first
        let for_each_result = process_rule(document, for_each_item)?;
        
        // If there's a map-item rule, process it for each item
        if let Some(_map_item) = &for_each_item.map_item {
            let mut mapped_results = Vec::new();
            
            // Get the array of items from for-each-item
            let items = match for_each_result {
                Value::Array(arr) => arr,
                _ => vec![for_each_result],
            };
            
            // For each item, process the map-item rule
            for item in items {
                if let Some(day_str) = item.as_str() {
                    // Find the specific day's items by looking for the day in the HTML
                    let day_items = find_items_for_day(document, day_str)?;
                    mapped_results.push(Value::Array(day_items));
                }
            }
            
            return Ok(Value::Array(mapped_results));
        }
        
        return Ok(for_each_result);
    }
    
    // Handle Object extract type with children/fields
    if let ExtractType::Object = &rule.extract_type {
        let children = rule.children.as_ref().or_else(|| rule.fields.as_ref());
        if let Some(children_rules) = children {
            // Use a specialized XPath-to-CSS converter for the specific patterns
            let selector_str = xpath_to_css_selector(&rule.xpath)?;
            let selector = Selector::parse(&selector_str)
                .map_err(|e| anyhow::anyhow!("Failed to parse selector: {}", e))?;
            
            let mut results = Vec::new();
            
            // For each matching element, process the children rules
            for element in document.select(&selector) {
                let mut object_result = serde_json::Map::new();
                
                // Process each child rule within the context of this element
                for child_rule in children_rules {
                    let child_selector_str = xpath_to_css_selector(&child_rule.xpath)?;
                    let child_selector = Selector::parse(&child_selector_str)
                        .map_err(|e| anyhow::anyhow!("Failed to parse child selector: {}", e))?;
                    
                    let child_value = match &child_rule.extract_type {
                        ExtractType::Text => {
                            let mut texts = Vec::new();
                            for child_element in element.select(&child_selector) {
                                let text = child_element.text().collect::<String>().trim().to_string();
                                if !text.is_empty() {
                                    texts.push(Value::String(text));
                                }
                            }
                            if texts.len() == 1 {
                                texts.into_iter().next().unwrap_or(Value::Null)
                            } else {
                                Value::Array(texts)
                            }
                        }
                        ExtractType::Attribute => {
                            let mut attrs = Vec::new();
                            for child_element in element.select(&child_selector) {
                                if let Some(attr_name) = &child_rule.attribute {
                                    if let Some(attr_value) = child_element.value().attr(attr_name) {
                                        attrs.push(Value::String(attr_value.to_string()));
                                    }
                                }
                            }
                            if attrs.len() == 1 {
                                attrs.into_iter().next().unwrap_or(Value::Null)
                            } else {
                                Value::Array(attrs)
                            }
                        }
                        ExtractType::Html => {
                            let mut htmls = Vec::new();
                            for child_element in element.select(&child_selector) {
                                htmls.push(Value::String(child_element.html()));
                            }
                            if htmls.len() == 1 {
                                htmls.into_iter().next().unwrap_or(Value::Null)
                            } else {
                                Value::Array(htmls)
                            }
                        }
                        ExtractType::Count => {
                            Value::Number(serde_json::Number::from(element.select(&child_selector).count()))
                        }
                        ExtractType::Object => Value::Null, // Nested objects not yet supported
                    };
                    
                    object_result.insert(child_rule.name.clone(), child_value);
                }
                
                results.push(Value::Object(object_result));
            }
            
            return if results.len() == 1 {
                Ok(results.into_iter().next().unwrap_or(Value::Null))
            } else {
                Ok(Value::Array(results))
            };
        } else {
            return Err(anyhow::anyhow!("Object extract type requires 'children' or 'fields'"));
        }
    }
    
    // Use a specialized XPath-to-CSS converter for the specific patterns
    let selector_str = xpath_to_css_selector(&rule.xpath)?;
    let selector = Selector::parse(&selector_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse selector: {}", e))?;
    
    match &rule.extract_type {
        ExtractType::Text => {
            let mut results = Vec::new();
            for element in document.select(&selector) {
                let text = if rule.xpath.ends_with("/text()") {
                    // For XPath ending with /text(), get the direct text content
                    element.text().collect::<String>().trim().to_string()
                } else {
                    // For other cases, get all text content including nested elements
                    element.text().collect::<String>().trim().to_string()
                };
                
                if !text.is_empty() {
                    results.push(Value::String(text));
                }
            }
            
            if results.len() == 1 {
                Ok(results.into_iter().next().unwrap_or(Value::Null))
            } else {
                Ok(Value::Array(results))
            }
        }
        ExtractType::Attribute => {
            let mut results = Vec::new();
            for element in document.select(&selector) {
                if let Some(attr_name) = &rule.attribute {
                    if let Some(attr_value) = element.value().attr(attr_name) {
                        results.push(Value::String(attr_value.to_string()));
                    }
                }
            }
            
            if results.len() == 1 {
                Ok(results.into_iter().next().unwrap_or(Value::Null))
            } else {
                Ok(Value::Array(results))
            }
        }
        ExtractType::Html => {
            let mut results = Vec::new();
            for element in document.select(&selector) {
                let html = element.html();
                results.push(Value::String(html));
            }
            
            if results.len() == 1 {
                Ok(results.into_iter().next().unwrap_or(Value::Null))
            } else {
                Ok(Value::Array(results))
            }
        }
        ExtractType::Count => {
            let count = document.select(&selector).count();
            Ok(Value::Number(serde_json::Number::from(count)))
        }
        ExtractType::Object => {
            // This should have been handled above, but just in case
            Err(anyhow::anyhow!("Object extract type must have 'children' or 'fields' defined"))
        }
    }
}

fn find_items_for_day(document: &Html, day: &str) -> Result<Vec<Value>> {
    find_items_for_day_in_month(document, day, None)
}

fn find_items_for_day_in_month(document: &Html, day: &str, month: Option<&str>) -> Result<Vec<Value>> {
    use scraper::Selector;
    
    let mut items = Vec::new();
    
    // Find all rows in the calendar table
    let table_selector = Selector::parse("table").unwrap();
    let row_selector = Selector::parse("tr").unwrap();
    let day_num_selector = Selector::parse("td.caltabletdnum").unwrap();
    let item_selector = Selector::parse("td.caltabletdevt").unwrap();
    let link_selector = Selector::parse("a").unwrap();
    
    // Find the calendar table
    for table in document.select(&table_selector) {
        let table_text = table.text().collect::<String>();
        if table_text.contains("Ex-Dividend Calendar") {
            // If month is specified, only process tables for that month
            if let Some(month_name) = month {
                if !table_text.contains(month_name) {
                    continue;
                }
            }
            let rows: Vec<_> = table.select(&row_selector).collect();
            
            for i in 0..rows.len() {
                let row = &rows[i];
                
                // Check if this row contains day numbers
                let day_nums: Vec<String> = row.select(&day_num_selector)
                    .map(|el| el.text().collect::<String>().trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                    
                if day_nums.contains(&day.to_string()) {
                    // Find the column index for this specific day
                    if let Some(day_index) = day_nums.iter().position(|d| d == day) {
                        // Look for the immediately following row that contains items for this day
                        if i + 1 < rows.len() {
                            let item_row = &rows[i + 1];
                            
                            // Check if this row contains item data (has caltabletdevt cells)
                            let item_cells: Vec<Vec<String>> = item_row.select(&item_selector)
                                .map(|cell| {
                                    cell.select(&link_selector)
                                        .map(|link| link.text().collect::<String>().trim().to_string())
                                        .filter(|s| !s.is_empty())
                                        .collect::<Vec<String>>()
                                })
                                .collect();
                            
                            // If this row has item cells and the column index is valid
                            if day_index < item_cells.len() && !item_cells[day_index].is_empty() {
                                for item in &item_cells[day_index] {
                                    items.push(Value::String(item.clone()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    Ok(items)
}

fn xpath_to_css_selector(xpath: &str) -> Result<String> {
    let xpath = xpath.trim();
    
    // Handle the specific XPath patterns from your dividend configuration
    if xpath.contains("//table[contains(., 'Ex-Dividend Calendar')]//th[contains(@style, 'font-size: 26px')]") {
        return Ok("table th[style*=\"font-size: 26px\"]".to_string());
    }
    
    if xpath.contains("//table[contains(., 'Ex-Dividend Calendar')]//td[contains(@class,'caltabletdevt')][.//span[@style=\"color: #4B9830; font-size: 22px;\"]]/../preceding-sibling::tr[1]/td[contains(@class,'caltabletdnum')]") {
        // This is a complex XPath that finds the preceding sibling row's day number
        // We'll use a simpler approach: find all td.caltabletdnum elements
        return Ok("td.caltabletdnum".to_string());
    }
    
    if xpath.contains("//table[contains(., 'Ex-Dividend Calendar')]//tr[td[@class='caltabletdnum']]/following-sibling::tr[1][td[@class='caltabletdevt']]") {
        // This finds the rows that contain both day numbers and their corresponding stocks
        // We'll use a simpler approach: find all tr elements and filter them in the processing
        return Ok("tr".to_string());
    }
    
    if xpath.contains("//table[contains(., 'Ex-Dividend Calendar')]//td[contains(@class,'caltabletdevt')][.//span[@style=\"color: #4B9830; font-size: 22px;\"]]") {
        // This finds the td elements that contain the stock symbols
        if xpath.ends_with("/text()") {
            // For the stocks XPath with /text(), we need to target the anchor tags
            return Ok("td.caltabletdevt span[style*=\"color: #000000\"] a".to_string());
        } else {
            return Ok("td.caltabletdevt span[style*=\"color: #4B9830\"][style*=\"font-size: 22px\"]".to_string());
        }
    }
    
    // General XPath to CSS conversion for simpler patterns
    let mut css = xpath.to_string();
    
    // Handle contains() function FIRST - convert to CSS attribute selectors
    // This must be done before removing @ symbols
    if css.contains("contains(") {
        // Convert contains(concat(' ', @class, ' '), ' classname ') to CSS class selector
        let concat_pattern = r#"contains\(concat\(' ', @(\w+), ' '\), ' ([^']+) '\)"#;
        let re_contains_concat = regex::Regex::new(concat_pattern).unwrap();
        css = re_contains_concat.replace_all(&css, |caps: &regex::Captures| {
            let attr = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            // For class attribute, convert to CSS class selector
            if attr == "class" {
                format!(".{}", value.replace(" ", "."))
            } else {
                format!("[{}*=\"{}\"]", attr, value)
            }
        }).to_string();
        
        // Convert contains(@attribute, 'value') to CSS attribute selector
        let attr_pattern = r#"contains\(@(\w+), ' ([^']+) '\)"#;
        let re_contains_attr = regex::Regex::new(attr_pattern).unwrap();
        css = re_contains_attr.replace_all(&css, |caps: &regex::Captures| {
            let attr = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if attr == "class" {
                format!(".{}", value.replace(" ", "."))
            } else {
                format!("[{}*=\"{}\"]", attr, value)
            }
        }).to_string();
        
        // Convert contains(., 'text') to a more generic selector
        if css.contains("contains(., ") {
            // Extract the element name and use it as a basic selector
            let parts: Vec<&str> = css.split_whitespace().collect();
            if let Some(first_part) = parts.first() {
                if first_part.contains("[") {
                    let element_name = first_part.split('[').next().unwrap_or("body");
                    css = element_name.to_string();
                }
            }
        }
    }
    
    // Remove leading // and replace with space
    if css.starts_with("//") {
        css = css[2..].to_string();
    }
    
    // Handle .// (current context) - replace with just space to maintain descendant relationship
    css = css.replace(".//", " ");
    
    // Replace // with space (descendant selector)
    css = css.replace("//", " ");
    
    // Handle @attribute extraction (e.g., /@href) BEFORE replacing / with space
    // The attribute name will be extracted during processing using rule.attribute, not by CSS selector
    let attr_pattern = r#"/@(\w+)"#;
    let re_attr = regex::Regex::new(attr_pattern).unwrap();
    css = re_attr.replace_all(&css, "").to_string();
    
    // Replace / with space (child selector)
    css = css.replace("/", " ");
    
    // Trim leading/trailing spaces and normalize multiple spaces
    css = css.trim().to_string();
    while css.contains("  ") {
        css = css.replace("  ", " ");
    }
    
    // Remove any remaining standalone @ symbols
    css = css.replace("@", "");
    
    // Trim again after removing @ symbols
    css = css.trim().to_string();
    
    // Handle 'and' in attribute selectors - convert to multiple attribute selectors
    css = css.replace(" and ", "][");
    
    // Convert element[.class] to element.class (class selectors should not use bracket notation)
    let class_bracket_pattern = r#"(\w+)\[\.([^\]]+)\]"#;
    let re_class_bracket = regex::Regex::new(class_bracket_pattern).unwrap();
    css = re_class_bracket.replace_all(&css, |caps: &regex::Captures| {
        let element = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let class = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        format!("{}.{}", element, class)
    }).to_string();
    
    // Replace single quotes with double quotes
    css = css.replace("'", "\"");
    
    // Handle numeric selectors like td[1], tr[2] - convert to nth-child
    css = css.replace("td[1]", "td:nth-child(1)");
    css = css.replace("td[2]", "td:nth-child(2)");
    css = css.replace("tr[1]", "tr:nth-child(1)");
    css = css.replace("tr[2]", "tr:nth-child(2)");
    css = css.replace("tr[3]", "tr:nth-child(3)");
    css = css.replace("tr[4]", "tr:nth-child(4)");
    css = css.replace("tr[5]", "tr:nth-child(5)");
    css = css.replace("tr[6]", "tr:nth-child(6)");
    css = css.replace("tr[7]", "tr:nth-child(7)");
    css = css.replace("tr[8]", "tr:nth-child(8)");
    css = css.replace("tr[9]", "tr:nth-child(9)");
    css = css.replace("tr[10]", "tr:nth-child(10)");
    css = css.replace("tr[11]", "tr:nth-child(11)");
    css = css.replace("tr[12]", "tr:nth-child(12)");
    css = css.replace("tr[13]", "tr:nth-child(13)");
    css = css.replace("tr[14]", "tr:nth-child(14)");
    css = css.replace("tr[15]", "tr:nth-child(15)");
    css = css.replace("tr[16]", "tr:nth-child(16)");
    css = css.replace("tr[17]", "tr:nth-child(17)");
    css = css.replace("tr[18]", "tr:nth-child(18)");
    css = css.replace("font[1]", "font:nth-child(1)");
    css = css.replace("font[2]", "font:nth-child(2)");
    
    // Handle /text() at the end - remove it and target the parent element
    if css.ends_with("/text()") {
        css = css.replace("/text()", "");
    }
    
    // Handle /text() in the middle of the path
    css = css.replace("/text()", "");
    
    // Handle text() at the end
    css = css.replace(" text()", "");
    
    Ok(css)
}
