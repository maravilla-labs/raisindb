"""Setup configuration for RaisinDB Python client."""

from setuptools import setup, find_packages

with open("README.md", "r", encoding="utf-8") as fh:
    long_description = fh.read()

setup(
    name="raisindb-client",
    version="0.1.0",
    author="RaisinDB Team",
    author_email="team@raisindb.com",
    description="Python client for RaisinDB - A Git-like document database",
    long_description=long_description,
    long_description_content_type="text/markdown",
    url="https://github.com/raisindb/raisindb",
    packages=find_packages(),
    classifiers=[
        "Development Status :: 3 - Alpha",
        "Intended Audience :: Developers",
        "License :: OSI Approved :: MIT License",
        "Programming Language :: Python :: 3",
        "Programming Language :: Python :: 3.9",
        "Programming Language :: Python :: 3.10",
        "Programming Language :: Python :: 3.11",
        "Programming Language :: Python :: 3.12",
    ],
    python_requires=">=3.9",
    install_requires=[
        "websockets>=12.0",
        "msgpack>=1.0.0",
        "typing-extensions>=4.0.0",
    ],
    extras_require={
        "dev": [
            "pytest>=7.0",
            "pytest-asyncio>=0.21.0",
            "black>=23.0",
            "mypy>=1.0",
            "ruff>=0.1.0",
        ],
    },
)
