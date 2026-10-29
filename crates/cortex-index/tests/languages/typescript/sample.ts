// Billing: a small module that uses each construct the TypeScript queries read.
import type { Money } from './money';
import fs = require('fs');
import * as geo from './geo';
export type { Rate } from './rates';

export interface Priced extends Comparable<Priced>, geo.Located {
  price(currency: string): Money;
  readonly label: Label;
}

export type Total<T> = Money | Partial<T>;

enum Status { Open, Paid }

namespace Billing {
  export const RATE = 0.2;
  export function tax(amount: Money): Money {
    return scale(amount, RATE);
  }
}

abstract class Shape implements Priced {
  abstract area(): number;
}

export class Invoice extends Base<Row> implements Priced, geo.Located {
  private notify = (event: Event) => emit(event);

  constructor(private store: Store) {
    super();
  }

  price(currency: string): Money {
    return new Money(this.total(), currency);
  }
}

export function parse(text: string): Invoice;
export function parse(text: Buffer): Invoice;
export function parse(input: unknown): Invoice {
  return Invoice.from(fs.readFileSync(input));
}

const cache: Map<string, Invoice> = new Map();

declare function emit(event: Event): void;
