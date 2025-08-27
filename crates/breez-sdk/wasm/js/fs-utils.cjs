/**
 * File system utilities for WASM bindgen in Node.js environment
 */

const fs = require("fs");
const path = require("path");

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

/**
 * Create a directory and all its parent directories if they don't exist
 * @param {string} dirPath - Path to the directory to create
 * @returns {Promise<void>}
 */
function createDirAll(dirPath) {
  return new Promise((resolve, reject) => {
    fs.mkdir(dirPath, { recursive: true }, (err) => {
      if (err) {
        reject(
          new Error(`Failed to create directory '${dirPath}': ${err.message}`)
        );
      } else {
        resolve();
      }
    });
  });
}

module.exports = {
  removeDirAll,
  createDirAll,
};
