#!/bin/bash
for i in {1..30}; do
  if ss -tln | grep -q ':50051 '; then
    echo "Port 50051 is open!"
    exit 0
  fi
  sleep 0.5
done
echo "Timeout"
exit 1
