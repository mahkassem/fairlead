<?php

// The Composer launcher's tests, with no network and no PHPUnit: `php tests-php/run.php`.
// Each *Test.php file registers cases with test(); a failed check throws.

declare(strict_types=1);

require __DIR__ . '/../src-php/load.php';
require __DIR__ . '/helpers.php';

foreach (glob(__DIR__ . '/*Test.php') ?: [] as $file) {
    require $file;
}

$failed = 0;
foreach (Harness::$cases as [$name, $case]) {
    try {
        $case();
        echo "ok - {$name}\n";
    } catch (Skip $s) {
        echo "ok - {$name} # skip {$s->getMessage()}\n";
    } catch (\Throwable $e) {
        $failed++;
        echo "not ok - {$name}\n  " . str_replace("\n", "\n  ", get_class($e) . ': ' . $e->getMessage()) . "\n";
    } finally {
        Harness::cleanup();
    }
}
$total = count(Harness::$cases);
echo $failed === 0 ? "{$total} passed\n" : "{$failed} of {$total} failed\n";
exit($failed === 0 ? 0 : 1);
