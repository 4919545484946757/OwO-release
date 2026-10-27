'use strict';

const { createServer } = require('./server');

const port = Number(process.env.PORT) || 3000;
const server = createServer();

server.listen(port, () => {
  const address = server.address();
  const actualPort = address && typeof address === 'object' ? address.port : port;
  console.log(`api-service listening on http://localhost:${actualPort}`);
});

function shutdown(signal) {
  console.log(`received ${signal}, shutting down`);
  server.close(() => process.exit(0));
}

process.on('SIGINT', () => shutdown('SIGINT'));
process.on('SIGTERM', () => shutdown('SIGTERM'));
