<?php

declare(strict_types=1);

foreach (['Errors', 'Downloader', 'Version', 'Target', 'Cache', 'Fs', 'Extract', 'Installer', 'Runner', 'Launcher'] as $file) {
    require_once __DIR__ . "/{$file}.php";
}
