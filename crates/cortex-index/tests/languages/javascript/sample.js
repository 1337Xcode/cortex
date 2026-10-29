// Orders: a small module that uses each construct the JavaScript queries read.
import React, { useState, useMemo as memo, default as Base } from 'react';
import * as api from "./api";
import './styles.css';
export { format, parse as parseOrder } from './format';
export * from './types';
export * as helpers from './helpers';
const fs = require('fs');
const double = (n) => n * 2;
const LIMIT = 10;
export const NAMES = ['a', 'b'];
let counter = 0;

class Order extends Base {
  total = () => this.lines.length;

  constructor(lines) {
    super();
    this.lines = lines;
  }

  static from(json) {
    return new Order(JSON.parse(json));
  }

  get size() {
    return this.lines.length;
  }
}
// A rush order ships first.
class Rush extends models.Order {}

function* ids() {
  yield counter++;
}

export function ship(order) {
  const label = (o) => format(o);
  return api.send(label(order));
}

export default function App() {
  const [items] = useState([]);
  return <Widget items={memo(() => items)}><div /></Widget>;
}

var legacy = function () {
  return import('./lazy');
};

module.exports.run = function () {};

const handlers = {
  click: () => ship(new Order([])),
  submit() {},
};
