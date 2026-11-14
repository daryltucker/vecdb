#!/usr/bin/env node
// A module in the shapes real JavaScript is written in: ES imports and
// exports, CommonJS require, classes with methods, arrow-function bindings,
// and top-level work.
import { readFile } from "node:fs/promises";
import Database from 'better-sqlite3';

const path = require("path");

/** Fetch rows for a page. */
export async function fetchPage(db, page) {
  const sql = "SELECT * FROM rows LIMIT ? OFFSET " + page * 10;
  return db.prepare(sql).all(10);
}

// Arrow binding: a function, named by its binding.
export const formatRow = (row) => `${row.id}: ${row.name}`;

// Not a function: a value computed with an arrow inside it.
const doubled = [1, 2, 3].map((x) => x * 2);

export default class Store {
  #db;

  constructor(file) {
    this.#db = new Database(file);
  }

  static open(file) {
    return new Store(file);
  }

  async load(name) {
    const text = await readFile(path.join("data", name), "utf8");
    return JSON.parse(text);
  }
}

function* ids() {
  yield 1;
}

if (process.env.DEBUG) {
  console.log("debug", doubled);
}

export { ids };
