#!/bin/bash

docker pull python:3.12-slim

docker build -t patch-test:a -f Dockerfile.a .
docker save patch-test:a > a.tar
docker inspect patch-test:a > inspect.json
docker image rm patch-test:a

docker build -t patch-test:b -f Dockerfile.b .
docker save patch-test:b > b.tar
docker image rm patch-test:b