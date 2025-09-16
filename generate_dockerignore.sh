#!/bin/bash

# Script to collect entries from all .gitignore files and add them to .dockerignore
# while respecting their relative paths

WORKSPACE_ROOT="."
DOCKERIGNORE_FILE="$WORKSPACE_ROOT/.dockerignore"

# Create a backup of the current .dockerignore
if [ -f "$DOCKERIGNORE_FILE" ]; then
  echo "Creating backup of existing .dockerignore"
  mv "$DOCKERIGNORE_FILE" "$DOCKERIGNORE_FILE.bak"
fi

# Create or clear the .dockerignore file
echo "# This file is auto-generated from all .gitignore files in the workspace" > "$DOCKERIGNORE_FILE"
echo "# Last updated: $(date)" >> "$DOCKERIGNORE_FILE"
echo "" >> "$DOCKERIGNORE_FILE"

# Always add .git folder to .dockerignore
echo "# Always ignore .git folder in Docker builds" >> "$DOCKERIGNORE_FILE"
echo ".git/" >> "$DOCKERIGNORE_FILE"
echo "" >> "$DOCKERIGNORE_FILE"

# Function to process a .gitignore file and add its contents to .dockerignore
process_gitignore() {
  local gitignore_path="$1"
  local relative_dir=$(dirname "${gitignore_path#$WORKSPACE_ROOT/}")
  
  echo "Processing $gitignore_path"
  
  # Add a comment to indicate which .gitignore file we're including
  echo "# From: $relative_dir/.gitignore" >> "$DOCKERIGNORE_FILE"
  
  # Read the gitignore file line by line
  while IFS= read -r line || [[ -n "$line" ]]; do
    # Skip empty lines and comments
    if [[ -z "$line" || "$line" =~ ^# ]]; then
      echo "$line" >> "$DOCKERIGNORE_FILE"
      continue
    fi
    
    # Process the ignore pattern
    if [[ "$relative_dir" == "." ]]; then
      # For root .gitignore, add the pattern as is
      echo "$line" >> "$DOCKERIGNORE_FILE"
      
      # If it appears to be a directory pattern (ends with / or could be a directory name)
      if [[ "$line" == */ || "$line" =~ ^[^*?]+$ && ! "$line" =~ \. ]]; then
        # Trim trailing slash if present
        dir_pattern=${line%/}
        # Add pattern to match all files within this directory
        echo "$dir_pattern/**/*" >> "$DOCKERIGNORE_FILE"
      fi
    else
      # For other .gitignore files, prefix with the relative directory
      # Handle patterns that already start with /
      local pattern
      if [[ "$line" == /* ]]; then
        pattern="$relative_dir$line"
      else
        pattern="$relative_dir/$line"
      fi
      
      echo "$pattern" >> "$DOCKERIGNORE_FILE"
      
      # If it appears to be a directory pattern (ends with / or could be a directory name)
      if [[ "$line" == */ || "$line" =~ ^[^*?]+$ && ! "$line" =~ \. ]]; then
        # Trim trailing slash if present
        dir_pattern=${pattern%/}
        # Add pattern to match all files within this directory
        echo "$dir_pattern/**/*" >> "$DOCKERIGNORE_FILE"
      fi
    fi
  done < "$gitignore_path"
  
  # Add an empty line after each .gitignore inclusion
  echo "" >> "$DOCKERIGNORE_FILE"
}

# Find all .gitignore files in the workspace and process them
find "$WORKSPACE_ROOT" -type f -name ".gitignore" | while read -r gitignore_file; do
  process_gitignore "$gitignore_file"
done

echo "Updated $DOCKERIGNORE_FILE with entries from all .gitignore files"
