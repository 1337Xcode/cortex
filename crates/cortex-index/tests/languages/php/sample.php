<?php
// Orders: a small file that uses each construct the PHP queries read.
namespace Shop\Orders;

use Shop\Models\Item;
use Shop\Support\{Money, Clock as Time};
require_once __DIR__ . '/helpers.php';

const RATE = 2;
define('LIMIT', 3);

interface Priced extends Named
{
    public function price(): int;
}

trait Logs
{
    public function log(string $message): void { error_log($message); }
}

#[Entity]
final class Order extends Base implements Priced
{
    use Logs;
    public const SLOTS = 4;
    private ?Item $item = null;

    public function __construct(private int $size) { parent::__construct($size); }

    public function price(): int
    {
        $this->log('price');
        return Money::round(fee($this->size)) + count(new Receipt());
    }
}

function fee(int $cents): int
{
    return intdiv($cents, 100);
}
