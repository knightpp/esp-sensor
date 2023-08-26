# esp-sensor

## Description

I'd like to use ESP microcontroller to collect data from sensors (DHT22) and send it to a database.
I've tried thingsboard but the Java overhead is too big and home-assistant which is good but not
sure how it stores the data.
So I chose InfluxDB, it uses only ~90MiB of RAM and has graphs/dashboards built-in!

## Hardware

- DHT22
- TM1637
- ESP32-C3

## Architecture

I use publish/subscribe model to easily add/remove functionality. There's a sensor reader thread
that publishes data, a data displayer thread and a data sender thread.
