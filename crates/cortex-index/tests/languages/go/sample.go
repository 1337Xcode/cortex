// Package shop: a small file that uses each construct the Go queries read.
package shop

import (
	"context"
	"fmt"
	str "strings"
	. "math"
	_ "embed"
)

import "net/http"

const Limit = 10

const (
	Open Status = iota
	Paid
)

type Status int

type ID = string

type Shop struct {
	orders []Order
	client *http.Client
}

type Store interface {
	Save(ctx context.Context, order Order) error
}

func (s *Shop) Pay(ctx context.Context, id ID) (Receipt, error) {
	order := s.find(id)
	return Receipt{Total: Sqrt(order.amount)}, fmt.Errorf("paid %s", str.ToUpper(id))
}

func (s Shop) find(id ID) Order {
	return s.orders[0]
}

func (q *Queue[T]) Push(item T) {}

func NewShop() *Shop {
	return &Shop{}
}
