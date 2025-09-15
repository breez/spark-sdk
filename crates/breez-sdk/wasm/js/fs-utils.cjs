/**
 * File system utilities for WASM bindgen in Node.js environment
 */

const fs = require("fs");

/**
 * Remove a directory and all its contents recursively
 * @param {string} dirPath - Path to the directory to remove
 * @returns {Promise<void>}
 */
function removeDirAll(dirPath) {
  return new Promise((resolve, reject) => {
    fs.rm(dirPath, { recursive: true, force: true }, (err) => {
      if (err) {
        // Ignore error if directory doesn't exist
        resolve();
      } else {
        resolve();
      }
    });
  });
}

module.exports = {
  removeDirAll,
};
