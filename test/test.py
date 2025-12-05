# -*- coding: utf-8 -*-
from loguru import logger

try:
    import numpy
    logger.success("Numpy Installed")
except ImportError:
    logger.error("Numpy Uninstalled")
