"""Settings from `app.ini`, which sits beside the log files it applies to."""

import configparser
import os
from dataclasses import dataclass

FILE_NAME = "app.ini"


@dataclass
class Settings:
    marker: str = "ERROR"


def load(log_path: str) -> Settings:
    """The settings for the log file at `log_path`: its directory's `app.ini`, if any."""
    parser = configparser.ConfigParser()
    parser.read(os.path.join(os.getcwd(), FILE_NAME), encoding="utf-8")
    return Settings(marker=parser.get("report", "marker", fallback="ERROR"))
