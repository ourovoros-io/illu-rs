use crate::db::Database;
use crate::server::tools::context;

pub fn handle_batch_context(
    db: &Database,
    symbols: &[String],
    full_body: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if symbols.is_empty() {
        return Ok("No symbols provided.".to_string());
    }

    let mut output = String::new();
    for (i, symbol) in symbols.iter().enumerate() {
        if i > 0 {
            output.push_str("\n---\n\n");
        }
        let result = context::handle_context(db, symbol, full_body, None)?;
        output.push_str(&result);
    }

    Ok(output)
}
