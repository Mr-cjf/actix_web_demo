pipeline {
	agent any

    stages {
		stage('Checkout') {
			steps {
				git branch: 'main', url: 'https://github.com/Mr-cjf/actix_web_demo.git'
            }
        }

        stage('Build Docker Image') {
			steps {
				sh 'docker build --cache-from web_demo:latest -t web_demo:latest .'
            }
        }

        stage('Deploy to Docker Swarm') {
			steps {
				sh 'docker stack deploy -c docker-stack.yml web'
            }
        }
    }
}
