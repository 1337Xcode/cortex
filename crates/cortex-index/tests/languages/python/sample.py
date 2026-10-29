"""Orders: a small module that uses each construct the Python queries read."""

from __future__ import annotations

import os.path
import json as js, sys
from typing import Optional
from . import helpers
from ..models import Order as OrderModel, Line
from .shapes import *

MAX_ITEMS = 50
DEFAULT_NAME: str = "order"
pending: list[Order]
first, second = 1, 2

type OrderId = int


def total(lines: list[Line], tax: Optional[float] = None) -> float:
    subtotal = sum(line.price for line in lines)
    return round(subtotal * (1 + (tax or 0)), 2)


class Order(OrderModel, mixins.Audited, Generic[T], metaclass=Registry):
    """An order and its lines."""

    limit = MAX_ITEMS

    def __init__(self, order_id: OrderId, lines: list[Line] | None = None) -> None:
        super().__init__(order_id)
        self.lines = lines or []

    @property
    def size(self) -> int:
        return len(self.lines)

    @helpers.cached
    def price(self) -> float:
        return total(self.lines)

    @staticmethod
    def parse(text: str | bytes) -> "Order":
        def clean(part):
            return part.strip()

        return Order(js.loads(clean(text))["id"])

    class Meta:
        table = "orders"


def ship(order: Order, *, to: Path | str | None) -> None: pass


on_error = lambda error: log(error)
