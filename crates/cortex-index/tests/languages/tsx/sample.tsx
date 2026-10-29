// Cart: a small component that uses the constructs TSX adds to TypeScript.
import React, { useState } from 'react';
import type { Item } from './item';

interface CartProps {
  items: Item[];
}

export function Cart({ items }: CartProps) {
  const [open, setOpen] = useState<boolean>(false);
  return (
    <Layout.Panel title="Cart">
      <ItemList items={items} onPick={(item: Item) => setOpen(!open)} />
      <button onClick={() => checkout(items)}>Pay</button>
    </Layout.Panel>
  );
}

export const Badge = ({ count }: { count: number }) => <span>{count}</span>;
