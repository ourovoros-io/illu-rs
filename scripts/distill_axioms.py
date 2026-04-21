import os
import glob
import json
import openai
from pathlib import Path

# Ensure you have OPENAI_API_KEY set in your environment
# pip install openai

SYSTEM_PROMPT = """
You are an expert Rust software architect. I am going to give you a section from a Rust textbook.
Your task is to extract the core 'Axioms' (inflexible rules, design patterns, and safety constraints) taught in this text.

For every distinct rule you find, output a JSON object matching this exact schema:
{ 
  "id": "String", 
  "category": "String", 
  "triggers": ["String"], 
  "rule_summary": "String", 
  "prompt_injection": "String (Start with MANDATORY RULE:)", 
  "anti_pattern": "String (Rust code showing the wrong way)", 
  "good_pattern": "String (Rust code showing the right way)" 
}

Ignore conversational text, history, or basic definitions. Only extract actionable architectural or syntactic rules. Return the result as a JSON array of these objects.
"""

def chunk_markdown(content, max_length=6000):
    """
    Split markdown into chunks by heading, attempting to keep them under max_length characters.
    """
    chunks = []
    current_chunk = []
    current_length = 0

    lines = content.split('\n')
    for line in lines:
        if line.startswith('## ') and current_length > 1000:
            chunks.append('\n'.join(current_chunk))
            current_chunk = []
            current_length = 0
            
        current_chunk.append(line)
        current_length += len(line) + 1

    if current_chunk:
        chunks.append('\n'.join(current_chunk))

    return chunks

def extract_axioms(client, text_chunk):
    try:
        response = client.chat.completions.create(
            model="gpt-4o",
            response_format={"type": "json_object"},
            messages=[
                {"role": "system", "content": SYSTEM_PROMPT + "\n\nMake sure to return a JSON object with a single key 'axioms' containing the array of rules."},
                {"role": "user", "content": text_chunk}
            ],
            temperature=0.2
        )
        
        content = response.choices[0].message.content
        data = json.loads(content)
        return data.get("axioms", [])
    except Exception as e:
        print(f"Error extracting axioms: {e}")
        return []

def main():
    api_key = os.getenv("OPENAI_API_KEY")
    if not api_key:
        print("Error: OPENAI_API_KEY environment variable is not set.")
        return

    client = openai.OpenAI(api_key=api_key)
    
    book_src_dir = os.path.join("assets", "rust-book", "src")
    if not os.path.exists(book_src_dir):
        print(f"Error: {book_src_dir} does not exist. Did you clone the repo?")
        return

    md_files = glob.glob(os.path.join(book_src_dir, "**", "*.md"), recursive=True)
    all_axioms = []

    for file_path in md_files:
        print(f"Processing {file_path}...")
        with open(file_path, "r", encoding="utf-8") as f:
            content = f.read()

        chunks = chunk_markdown(content)
        for i, chunk in enumerate(chunks):
            if len(chunk.strip()) < 100: # skip very small chunks
                continue
                
            print(f"  Chunk {i+1}/{len(chunks)} ({len(chunk)} chars)...")
            axioms = extract_axioms(client, chunk)
            if axioms:
                print(f"  -> Found {len(axioms)} axioms.")
                all_axioms.extend(axioms)

    # Output the result
    out_file = os.path.join("assets", "axioms.json")
    with open(out_file, "w", encoding="utf-8") as f:
        json.dump(all_axioms, f, indent=2)
        
    print(f"Finished! Extracted {len(all_axioms)} axioms into {out_file}")

if __name__ == "__main__":
    main()
