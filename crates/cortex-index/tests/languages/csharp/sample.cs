// Orders: a small file that uses each construct the C# queries read.
using System;
using static System.Math;
using Json = System.Text.Json.JsonSerializer;

namespace Shop.Orders
{
    public delegate void Paid(Order order);

    public interface IPriced : INamed
    {
        decimal Price();
    }

    public enum Status { Open, Paid }

    public record Line(Item Item, int Count);

    [Serializable]
    public sealed class Order : Base<Item>, IPriced
    {
        public const int Limit = 3;
        private readonly List<Item> items = new List<Item>();

        public Order(int size) : base(size) { }

        public decimal Total { get; set; }

        public decimal Price()
        {
            int Fee(int cents) => cents / 100;
            items.Add(new Item());
            return Max(Fee(1), Helper.Round<decimal>(Total));
        }

        [Test]
        public void Pays() => Assert.AreEqual(3, Limit);
    }
}
