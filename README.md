# XPath to JSON CLI Tool

A generic Rust CLI tool that processes HTML using XPath configurations to extract structured JSON data in any format you specify.

## Features

- **Generic XPath Processing**: Works with any XPath expressions
- **Flexible Output Formats**: Define any JSON structure you want
- **Multiple Extraction Types**: Text, attributes, HTML content, and element counts
- **Encoding Support**: Handles various HTML encodings (UTF-8, Windows-1252, etc.)
- **Error Handling**: Comprehensive error reporting in output JSON
- **Agnostic Design**: No hardcoded assumptions about data types or structures

## Installation

```bash
cargo build --release
```

## Usage

```bash
./target/release/xpath-to-json -c config.json -h input.html [-o output.json]
```

### Arguments

- `-c, --config`: Path to the JSON configuration file
- `-h, --html`: Path to the HTML file to process
- `-o, --output`: Output file path (optional, defaults to stdout)

## Configuration Format

The configuration file should be a JSON file with the following structure:

```json
{
  "name": "Configuration Name",
  "description": "Optional description",
  "output_sample": [
    {
      "field1": "example_value1",
      "field2": "example_value2"
    }
  ],
  "rules": [
    {
      "name": "rule_name",
      "xpath": "//your/xpath/expression",
      "extract_type": "text|attribute|html|count",
      "attribute": "attribute_name (required for attribute type)",
      "iterate_over": "previous_rule_name (optional)",
      "children": [/* nested rules for iteration */]
    }
  ]
}
```

### Output Sample

The `output_sample` field defines the expected JSON structure for the output. The tool will automatically map extracted data to match this format. Examples:

- **Generic Data**: `[{"field1": "value1", "field2": "value2"}]`
- **News Articles**: `[{"title": "Breaking News", "date": "2025-01-15", "author": "John Doe"}]`
- **Products**: `[{"name": "iPhone 15", "price": "$999", "category": "Electronics"}]`
- **Any Custom Format**: Define any JSON structure you need

### Extract Types

- `text`: Extract text content from elements
- `attribute`: Extract attribute values (requires `attribute` field)
- `html`: Extract HTML content of elements
- `count`: Count matching elements

## Example

See the `examples/` directory for sample configuration and HTML files.

### Running the Example

```bash
cargo run -- -c examples/ex-dividend-config.json -h examples/sample-html.html
```

## XPath Support

Currently, the tool uses a simplified XPath-to-CSS selector conversion. For production use with complex XPath expressions, consider using a dedicated XPath library.

## Error Handling

The tool provides comprehensive error handling:
- Configuration file parsing errors
- HTML parsing errors
- XPath evaluation errors
- Output serialization errors

All errors are included in the output JSON for debugging.
